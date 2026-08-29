/*
 * esp32_cell.ino — the minimal exocortex cell (Rung 5a + 5b transport lane).
 *
 * ONE quilt value cell over ONE link, selected at compile time (see
 * qw_transport.h): USB-CDC serial @ 115200 (default), ESP-Now, or BLE.
 * The SAME QuiltWire v0 16-byte frame rides every road. Polls a
 * temperature-ish stub at 1 Hz, sends frames:
 *   DELTA when |v - last_sent| > EPS, else a TICK heartbeat every 30 s,
 *   ALARM (redundant fire tolerated) if the reading goes out of band,
 *   LINKMETA when the radio reports a fresh per-frame RSSI observation.
 * Keeps its own seq; honest retry/backoff on write; dropped frames are
 * counted, never faked — the resulting seq gap IS the reliability signal
 * the desktop peer reads (observed, not declared).
 *
 * DEPENDS ON: quiltwire.h (pure C codec, byte-identical to the Rust
 * link-core; host-tested in test_quiltwire.c) and qw_transport.h
 * (compile-time transport select).
 *
 * TARGETS: any ESP32 devkit. Classic parts: Serial = UART0 through the
 * USB-UART bridge (baud honored). S3/C3 with native USB: Serial = USB CDC
 * (baud ignored by CDC — begin(115200) is harmless and keeps one code path).
 *
 * BUILD (Arduino IDE): install "esp32 by Espressif Systems" boards package,
 * select your board, open this sketch (quiltwire.h and qw_transport.h must
 * sit next to it), Upload, open Serial Monitor at 115200. PlatformIO
 * equivalent, ESP-Now flavor:
 *   [env:cell] platform = espressif32; framework = arduino
 *   build_flags = -D QW_TRANSPORT=QW_TRANSPORT_ESPNOW
 *
 * STATUS: *** UNTESTED ON SILICON *** — no board attached. The codec is
 * host-tested (test_quiltwire.c, gcc) and the USB-CDC byte path is proven
 * end-to-end by the pty loopback test; the Arduino/ESP-Now/BLE glue
 * (Serial CDC behavior, esp_now callbacks, Bluedroid BLE, timing) is
 * written to be reviewed by eye and verified the moment hardware arrives.
 * No claims beyond that.
 *
 * WHAT IS DELIBERATELY ABSENT (per LINK-LAYER-FEASIBILITY.md §4.2):
 *   - no timestamps-in-us (no cross-clock claims; the receiver stamps)
 *   - no routing headers (egocentric — no global addressing)
 *   - no encryption (v0; ESP-Now pairing is a later phase)
 *   - no TLVs on the serial link (bare 16-byte frames; TLV 0x01 rides only
 *     when >= 2 live links exist and the MTU allows — not this firmware)
 * CHANGED AT RUNG 5b: the cell now MAY declare one link metadata
 * observation — the radio's own per-frame RSSI, carried as the LINKMETA
 * value — on radio roads only. USB-CDC still declares nothing (there is
 * no radio to observe). The desktop stamps its own receiver-side RSSI
 * independently; the two observations are both data, neither is trusted
 * to speak for the other.
 */

#include "quiltwire.h"
#include "qw_transport.h"

// ---- cell identity ----
static const uint8_t  CELL_ID     = 7;        // demo universe has few cells

// ---- sender discipline (mirrored by the Rust twin in
//      crates/quilt-wire/tests/pty_loopback.rs `firmware_twin_frames`) ----
static const float    EPS             = 0.05f; // delta threshold (deg C)
static const uint32_t HEARTBEAT_TICKS = 30;    // TICK heartbeat cadence (s)
static const float    ALARM_HI        = 85.0f; // out-of-band threshold

// ---- honest retry/backoff ----
static const uint8_t  SEND_ATTEMPTS = 3;
static const uint32_t BACKOFF_START_MS = 8;
static const uint32_t BACKOFF_MAX_MS   = 512;

// ---- state ----
static uint16_t seq           = 0;     // wraps; gap = reliability observation
static uint32_t tick          = 0;     // sender-local 1 Hz tick
static float    last_sent     = NAN;   // last value actually sent
static uint32_t since_sent    = 0;     // ticks since last send
static uint32_t frames_sent   = 0;
static uint32_t frames_dropped = 0;
static int16_t  rssi_reported = 0;     // last RSSI declared via LINKMETA
static bool     rssi_declared = false; // no radio observation seen yet

/*
 * Temperature-ish sensor STUB. Honest about being a stub: no real sensor is
 * attached. It returns a plausible slow-staircase value (plateaus of 40 s,
 * steps of 0.5) so both the DELTA and the heartbeat paths are exercised on
 * silicon. Swap in a real sensor (e.g. DHT22) by replacing ONLY this
 * function — one file change, per the feasibility paper §5.
 */
static float sensor_stub(void)
{
    return 20.0f + (float)((tick / 40u) % 3u) * 0.5f;
}

/*
 * Send one QuiltWire frame over the selected transport with honest
 * retry/backoff. Returns true iff all 16 bytes were accepted by the road
 * (queued, on the radios — see qw_transport.h); on failure after
 * SEND_ATTEMPTS tries, returns false and the caller advances seq anyway —
 * the gap is the signal, never a faked send.
 */
static bool send_frame(uint8_t kind, float value)
{
    qw_frame f;
    f.kind    = kind;
    f.cell_id = CELL_ID;
    f.seq     = seq;
    f.tick    = tick;
    f.value   = value;

    uint8_t buf[QW_FRAME_LEN];
    qw_encode(&f, buf);

    uint32_t delay_ms = BACKOFF_START_MS;
    for (uint8_t attempt = 0; attempt < SEND_ATTEMPTS; attempt++) {
        size_t n = transport_write(buf, QW_FRAME_LEN);
        if (n == QW_FRAME_LEN) {
            frames_sent++;
            return true;
        }
        // Short write on a wired stream is a wedged/bufferless port; zero on
        // a radio is a dead queue / no BLE peer. Back off, retry.
        delay(delay_ms);
        delay_ms = min(delay_ms * 4ul, BACKOFF_MAX_MS);
    }
    frames_dropped++;
    return false;
}

/*
 * If the radio has observed a per-frame RSSI since we last declared one,
 * fire a LINKMETA carrying it (dBm, as the f32 value). Wired serial never
 * fires: transport_rssi_dbm() returns false, and the road stays silent
 * about quality — observation happens receiver-side there.
 */
static void maybe_declare_rssi(void)
{
    int16_t r;
    if (transport_rssi_dbm(&r) && (!rssi_declared || r != rssi_reported)) {
        send_frame(QW_KIND_LINKMETA, (float)r);
        seq++;
        rssi_reported = r;
        rssi_declared = true;
    }
}

void setup()
{
    transport_begin();
    // One LINKMETA frame opens the session so the peer sees a genesis-ish
    // arrival before data; value carries the firmware version as float bits.
    send_frame(QW_KIND_LINKMETA, 0.2f); // fw v0.2 (rung 5b: transports)
    seq++; // a frame was consumed (sent or dropped — seq advances either way)
}

void loop()
{
    tick++;
    since_sent++;

    // Radio roads: declare a fresh RSSI observation if one arrived.
    maybe_declare_rssi();

    float v = sensor_stub();

    if (v >= ALARM_HI) {
        // ALARM: cost no object. Fire every tick while hot; duplicates are
        // fine and *which duplicates arrive* is field data.
        send_frame(QW_KIND_ALARM, v);
        seq++;
        last_sent = v;
        since_sent = 0;
        return; // alarm tick does not also send delta/heartbeat
    }

    bool changed = !isnan(last_sent) && (fabsf(v - last_sent) > EPS);
    if (isnan(last_sent)) {
        changed = true; // first real reading after boot
    }

    if (changed || since_sent >= HEARTBEAT_TICKS) {
        send_frame(changed ? QW_KIND_DELTA : QW_KIND_TICK, v);
        seq++;
        last_sent  = v;
        since_sent = 0;
    }

    delay(1000); // 1 Hz cell tick
}
