/*
 * qw_transport.h — QuiltWire transport abstraction for the ESP32 cell
 * (Rung 5b).
 *
 * ONE transport, selected at compile time; the SAME 16-byte QuiltWire v0
 * frame goes on the wire regardless:
 *
 *   QW_TRANSPORT_USB_CDC  (0, default) — wired serial, the Rung 5a road.
 *   QW_TRANSPORT_ESPNOW   (1) — Espressif ESP-Now (WiFi STA, no AP).
 *   QW_TRANSPORT_BLE      (2) — BLE peripheral, Nordic-UART-style service.
 *
 * Select with e.g. `-D QW_TRANSPORT=QW_TRANSPORT_ESPNOW` (PlatformIO
 * build_flags) or one #define above the #include in the sketch.
 *
 * Contract (all three transports):
 *   transport_begin()        — bring the road up (blocking, bounded).
 *   transport_write(b, n)    — returns bytes accepted by the road; 0 on a
 *                              wedged link. NOTE for the radios: "accepted"
 *                              means QUEUED (esp_now_send ok / BLE notify
 *                              issued), not ACKed by the peer — delivery
 *                              loss shows up as seq gaps receiver-side,
 *                              which is the honesty contract.
 *   transport_rssi_dbm(&v)   — true iff the radio has observed a per-frame
 *                              RSSI; wired serial always returns false.
 *                              ESP-Now: RSSI of the last inbound frame
 *                              (recv callback). BLE: controller-read RSSI
 *                              of the connected peer, refreshed on each
 *                              inbound write. This feeds the cell-side
 *                              LINKMETA observation; the desktop still
 *                              stamps its own receiver-side observation —
 *                              subtext stays observed, on BOTH sides.
 *
 * STATUS: *** UNTESTED ON SILICON *** — no board attached. The USB-CDC
 * path is host-proven by the pty loopback test (its byte stream is what
 * the desktop peer sees); the ESP-Now and BLE glue below is written to be
 * reviewed by eye and verified the moment hardware arrives. No claims
 * beyond that. Written against the Arduino-ESP32 2.x/3.x APIs (ESP-Now
 * recv callback taking esp_now_recv_info_t; Bluedroid BLEDevice).
 */
#ifndef QW_TRANSPORT_H
#define QW_TRANSPORT_H

#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdbool.h>

#define QW_TRANSPORT_USB_CDC 0
#define QW_TRANSPORT_ESPNOW  1
#define QW_TRANSPORT_BLE     2

#ifndef QW_TRANSPORT
#define QW_TRANSPORT QW_TRANSPORT_USB_CDC
#endif

/* Latest per-frame RSSI observation (radio roads only). Written from the
 * radio callback context; read from loop() — int16/bool reads are atomic
 * on Xtensa, and a torn pair only costs one stale LINKMETA. */
static volatile int16_t qw_rssi_dbm   = 0;
static volatile bool    qw_have_rssi  = false;

static bool transport_rssi_dbm(int16_t *out)
{
    if (!qw_have_rssi) return false;
    *out = qw_rssi_dbm;
    return true;
}

/* ============================ USB-CDC ============================ */
#if QW_TRANSPORT == QW_TRANSPORT_USB_CDC

static const uint32_t QW_SERIAL_BAUD = 115200ul;

static void transport_begin(void)
{
    Serial.begin(QW_SERIAL_BAUD);
    // Bounded wait for CDC host to open the port (S3 native USB). On
    // UART-bridge parts this returns immediately once ready.
    const uint32_t t0 = millis();
    while (!Serial && (millis() - t0) < 4000ul) {
        delay(10);
    }
}

static size_t transport_write(const uint8_t *buf, size_t len)
{
    return Serial.write(buf, len);
}

/* ============================ ESP-Now ============================ */
#elif QW_TRANSPORT == QW_TRANSPORT_ESPNOW

#include <WiFi.h>
#include <esp_now.h>

/* Peer MAC: default is broadcast (any ESP-Now receiver hears the frames).
 * Pin the portal's MAC here once the topology is real; pairing/security is
 * a later phase (frame v0 carries no encryption). */
#ifndef QW_ESPNOW_PEER_MAC
#define QW_ESPNOW_PEER_MAC { 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF }
#endif

static void qw_espnow_recv_cb(const esp_now_recv_info_t *info,
                              const uint8_t *data, int len)
{
    (void)data; (void)len; // the cell is a sender; inbound frames are
                           // observed for link metadata only
    if (info != NULL && info->rx_ctrl != NULL) {
        qw_rssi_dbm  = (int16_t)info->rx_ctrl->rssi;
        qw_have_rssi = true;
    }
}

static void transport_begin(void)
{
    WiFi.mode(WIFI_STA);
    WiFi.disconnect(); // ESP-Now rides the radio without an AP association
    if (esp_now_init() != ESP_OK) {
        // Honest failure: leave the road dead — every write will return 0
        // and the resulting seq gaps are the signal. No fake sends.
        return;
    }
    esp_now_register_recv_cb(qw_espnow_recv_cb);

    esp_now_peer_info_t peer;
    memset(&peer, 0, sizeof(peer));
    uint8_t mac[6] = QW_ESPNOW_PEER_MAC;
    memcpy(peer.peer_addr, mac, 6);
    peer.channel = 0;      // current channel
    peer.encrypt = false;  // v0: no encryption (per LINK-LAYER-FEASIBILITY)
    esp_now_add_peer(&peer);
}

static size_t transport_write(const uint8_t *buf, size_t len)
{
    uint8_t mac[6] = QW_ESPNOW_PEER_MAC;
    // ESP_OK means QUEUED for transmission, not delivered — see contract.
    return (esp_now_send(mac, buf, len) == ESP_OK) ? len : 0;
}

/* ============================== BLE ============================== */
#elif QW_TRANSPORT == QW_TRANSPORT_BLE

#include <BLEDevice.h>
#include <BLEServer.h>
#include <BLEUtils.h>
#include <BLE2902.h>
#include <esp_gap_ble_api.h>

/* Nordic-UART-style service: TX (notify) = cell -> desktop, RX (write) =
 * desktop -> cell. 16-byte frames fit the default ATT MTU (23) as a single
 * notification. */
#define QW_BLE_SERVICE_UUID "6E400001-B5A3-F393-E0A9-E50E24DCCA9E"
#define QW_BLE_TX_UUID      "6E400003-B5A3-F393-E0A9-E50E24DCCA9E"
#define QW_BLE_RX_UUID      "6E400002-B5A3-F393-E0A9-E50E24DCCA9E"

static BLECharacteristic *qw_ble_tx = NULL;
static volatile bool      qw_ble_connected = false;
static esp_bd_addr_t      qw_ble_peer_addr;

/* GAP handler: captures the controller's answer to esp_ble_gap_read_rssi —
 * the per-frame RSSI observation for LINKMETA. */
static void qw_ble_gap_cb(esp_gap_ble_cb_event_t event,
                          esp_ble_gap_cb_param_t *param)
{
    if (event == ESP_GAP_BLE_READ_RSSI_COMPLETE_EVT &&
        param->read_rssi_cmpl.status == ESP_BT_STATUS_SUCCESS) {
        qw_rssi_dbm  = (int16_t)param->read_rssi_cmpl.rssi;
        qw_have_rssi = true;
    }
}

class QwBleServerCbs : public BLEServerCallbacks {
    void onConnect(BLEServer *server, esp_ble_gatts_cb_param_t *param) override {
        qw_ble_connected = true;
        memcpy(qw_ble_peer_addr, param->connect.remote_bda, sizeof(esp_bd_addr_t));
    }
    void onDisconnect(BLEServer *server) override {
        qw_ble_connected = false;
        qw_have_rssi = false;
        server->getAdvertising()->start(); // re-advertise, honestly
    }
};

class QwBleRxCbs : public BLECharacteristicCallbacks {
    void onWrite(BLECharacteristic *ch) override {
        (void)ch; // payload unused; the write TRIGGERS an RSSI read
        if (qw_ble_connected) {
            esp_ble_gap_read_rssi(qw_ble_peer_addr); // answer -> qw_ble_gap_cb
        }
    }
};

static void transport_begin(void)
{
    BLEDevice::init("quilt-cell");
    BLEDevice::setCustomGapHandler(qw_ble_gap_cb);
    BLEServer *server = BLEDevice::createServer();
    server->setCallbacks(new QwBleServerCbs());
    BLEService *svc = server->createService(QW_BLE_SERVICE_UUID);
    qw_ble_tx = svc->createCharacteristic(
        QW_BLE_TX_UUID, BLECharacteristic::PROPERTY_NOTIFY);
    qw_ble_tx->addDescriptor(new BLE2902());
    BLECharacteristic *rx = svc->createCharacteristic(
        QW_BLE_RX_UUID,
        BLECharacteristic::PROPERTY_WRITE | BLECharacteristic::PROPERTY_WRITE_NR);
    rx->setCallbacks(new QwBleRxCbs());
    svc->start();
    BLEAdvertising *adv = BLEDevice::getAdvertising();
    adv->addServiceUUID(QW_BLE_SERVICE_UUID);
    adv->start();
}

static size_t transport_write(const uint8_t *buf, size_t len)
{
    if (!qw_ble_connected) return 0; // no peer: honest miss, seq advances
    qw_ble_tx->setValue(const_cast<uint8_t *>(buf), len);
    qw_ble_tx->notify(); // queued, not ACKed — see contract
    return len;
}

#else
#error "QW_TRANSPORT must be one of QW_TRANSPORT_USB_CDC / _ESPNOW / _BLE"
#endif

#endif /* QW_TRANSPORT_H */
