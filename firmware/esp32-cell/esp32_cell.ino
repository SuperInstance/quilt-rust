/*
 * esp32_cell.ino — the minimal exocortex cell (Rung 5a).
 *
 * ONE quilt value cell over ONE link (USB-CDC serial @ 115200). Polls a
 * temperature-ish stub at 1 Hz, sends QuiltWire v0 16-byte frames:
 *   DELTA when |v - last_sent| > EPS, else a TICK heartbeat every 30 s,
 *   ALARM (redundant fire tolerated) if the reading goes out of band.
 * Keeps its own seq; honest retry/backoff on write; dropped frames are
 * counted, never faked — the resulting seq gap IS the reliability signal
 * the desktop peer reads (observed, not declared).
 *
 * DEPENDS ON: quiltwire.h (pure C codec, byte-identical to the Rust
 * link-core; host-tested in test_quiltwire.c).
 *
 * TARGETS: any ESP32 devkit. Classic parts: Serial = UART0 through the
 * USB-UART bridge (baud honored). S3/C3 with native USB: Serial = USB CDC
 * (baud ignored by CDC — begin(115200) is harmless and keeps one code path).
 *
 * BUILD (Arduino IDE): install "esp32 by Espressif Systems" boards package,
 * select your board, open this sketch (quiltwire.h must sit next to it),
 * Upload, open Serial Monitor at 115200. PlatformIO equivalent:
 *   [env:cell] platform = espressif32; framework = arduino
 *
 * STATUS: *** UNTESTED ON SILICON *** — no board attached. The codec is
 * host-tested (test_quiltwire.c, gcc); the Arduino-specific glue below
 * (Serial CDC behavior, timing) is written to be reviewed by eye and
 * verified the moment hardware arrives. No claims beyond that.
 *
 * WHAT IS DELIBERATELY ABSENT (per LINK-LAYER-FEASIBILITY.md §4.2):
 *   - no timestamps-in-us (no cross-clock claims; the receiver stamps)
 *   - no sender link-quality self-reports (subtext is observed, not declared)
 *   - no routing headers (egocentric — no global addressing)
 *   - no encryption (v0; ESP-Now pairing is a later phase)
 *   - no TLVs on the serial link (bare 16-byte frames; TLV 0x01 rides only
 *     when >= 2 live links exist and the MTU allows — not this firmware)
 */

#include "quiltwire.h"

// ---- cell identity ----
static const uint8_t  CELL_ID     = 7;        // demo universe has few cells
static const uint32_t SERIAL_BAUD = 115200ul;

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
 * Send one QuiltWire frame over Serial with honest retry/backoff.
 * Returns true iff all 16 bytes were accepted by the stream; on failure
 * after SEND_ATTEMPTS tries, returns false and the caller advances seq
 * anyway — the gap is the signal, never a faked send.
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
        size_t n = Serial.write(buf, QW_FRAME_LEN);
        if (n == QW_FRAME_LEN) {
            frames_sent++;
            return true;
        }
        // Partial write on a stream API is a wedged/bufferless port
        // (host not listening, CDC not open). Back off, retry.
        delay(delay_ms);
        delay_ms = min(delay_ms * 4ul, BACKOFF_MAX_MS);
    }
    frames_dropped++;
    return false;
}

void setup()
{
    Serial.begin(SERIAL_BAUD);
    // Bounded wait for CDC host to open the port (S3 native USB). On
    // UART-bridge parts this returns immediately once ready.
    const uint32_t t0 = millis();
    while (!Serial && (millis() - t0) < 4000ul) {
        delay(10);
    }
    // One LINKMETA frame opens the session so the peer sees a genesis-ish
    // arrival before data; value carries the firmware version as float bits.
    send_frame(QW_KIND_LINKMETA, 0.1f); // fw v0.1
    seq++; // a frame was consumed (sent or dropped — seq advances either way)
}

void loop()
{
    tick++;
    since_sent++;

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
