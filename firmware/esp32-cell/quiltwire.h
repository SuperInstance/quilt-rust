/*
 * quiltwire.h — QuiltWire v0 frame codec in portable C99.
 *
 * Byte-identical to crates/quilt-wire/src/frame.rs (Rust link-core):
 *   byte  0    : magic 0x51 ('Q')
 *   byte  1    : version 0x01
 *   byte  2    : kind  0x00 TICK | 0x01 DELTA | 0x02 ALARM | 0x03 LINKMETA | 0x04 ACK
 *   byte  3    : cell id (u8)
 *   bytes 4-5  : seq (u16 LE, wraps)
 *   bytes 6-9  : tick (u32 LE, sender-local)
 *   bytes 10-13: value (f32 LE, raw IEEE-754 bits)
 *   bytes 14-15: CRC16-CCITT-FALSE over bytes 0..=13
 *                (poly 0x1021, init 0xFFFF, no reflection, xorout 0;
 *                 check("123456789") == 0x29B1)
 *
 * No Arduino dependencies — this header compiles on the ESP32 and on a
 * desktop host (see test_quiltwire.c, run under gcc in CI/sandbox).
 * Golden vector shared with the Rust tests:
 *   510101070201e80300000000ac41d6d3  (kind=DELTA cell=7 seq=0x0102
 *   tick=1000 value=21.5f)
 *
 * STATUS: compiled and roundtrip-tested on host (gcc). The ESP32 build is
 * UNTESTED ON SILICON — no board attached yet.
 */
#ifndef QUILTWIRE_H
#define QUILTWIRE_H

#include <stdint.h>
#include <string.h>

#define QW_MAGIC      0x51u
#define QW_VERSION    0x01u
#define QW_FRAME_LEN  16u
#define QW_CRC_SPAN   14u

#define QW_KIND_TICK     0x00u
#define QW_KIND_DELTA    0x01u
#define QW_KIND_ALARM    0x02u
#define QW_KIND_LINKMETA 0x03u
#define QW_KIND_ACK      0x04u

typedef struct {
    uint8_t  kind;      /* QW_KIND_* */
    uint8_t  cell_id;
    uint16_t seq;       /* wraps */
    uint32_t tick;      /* sender-local, 1 Hz on the cell */
    float    value;     /* raw bits pass through — any IEEE-754 pattern */
} qw_frame;

static uint16_t qw_crc16(const uint8_t *data, uint32_t len)
{
    uint16_t crc = 0xFFFFu;
    for (uint32_t i = 0; i < len; i++) {
        crc ^= (uint16_t)((uint16_t)data[i] << 8);
        for (uint8_t b = 0; b < 8; b++) {
            if (crc & 0x8000u) {
                crc = (uint16_t)((crc << 1) ^ 0x1021u);
            } else {
                crc = (uint16_t)(crc << 1);
            }
        }
    }
    return crc;
}

/* Encode f into out[16]. Returns QW_FRAME_LEN. */
static uint32_t qw_encode(const qw_frame *f, uint8_t out[QW_FRAME_LEN])
{
    uint32_t bits;
    memcpy(&bits, &f->value, 4); /* avoid strict-aliasing punning */

    out[0] = QW_MAGIC;
    out[1] = QW_VERSION;
    out[2] = f->kind;
    out[3] = f->cell_id;
    out[4] = (uint8_t)(f->seq & 0xFFu);
    out[5] = (uint8_t)((f->seq >> 8) & 0xFFu);
    out[6] = (uint8_t)(f->tick & 0xFFu);
    out[7] = (uint8_t)((f->tick >> 8) & 0xFFu);
    out[8] = (uint8_t)((f->tick >> 16) & 0xFFu);
    out[9] = (uint8_t)((f->tick >> 24) & 0xFFu);
    out[10] = (uint8_t)(bits & 0xFFu);
    out[11] = (uint8_t)((bits >> 8) & 0xFFu);
    out[12] = (uint8_t)((bits >> 16) & 0xFFu);
    out[13] = (uint8_t)((bits >> 24) & 0xFFu);

    uint16_t crc = qw_crc16(out, QW_CRC_SPAN);
    out[14] = (uint8_t)(crc & 0xFFu);
    out[15] = (uint8_t)((crc >> 8) & 0xFFu);
    return QW_FRAME_LEN;
}

/* Decode in[16] into *f. Returns 0 on success, negative on error. */
static int qw_decode(const uint8_t in[QW_FRAME_LEN], qw_frame *f)
{
    if (in[0] != QW_MAGIC || in[1] != QW_VERSION) {
        return -1; /* bad header */
    }
    if (in[2] > QW_KIND_ACK) {
        return -2; /* bad kind */
    }
    uint16_t stored = (uint16_t)(in[14] | ((uint16_t)in[15] << 8));
    uint16_t calc = qw_crc16(in, QW_CRC_SPAN);
    if (stored != calc) {
        return -3; /* CRC mismatch */
    }
    uint32_t bits = (uint32_t)in[10] | ((uint32_t)in[11] << 8)
                  | ((uint32_t)in[12] << 16) | ((uint32_t)in[13] << 24);
    f->kind = in[2];
    f->cell_id = in[3];
    f->seq = (uint16_t)(in[4] | ((uint16_t)in[5] << 8));
    f->tick = (uint32_t)in[6] | ((uint32_t)in[7] << 8)
            | ((uint32_t)in[8] << 16) | ((uint32_t)in[9] << 24);
    memcpy(&f->value, &bits, 4);
    return 0;
}

#endif /* QUILTWIRE_H */
