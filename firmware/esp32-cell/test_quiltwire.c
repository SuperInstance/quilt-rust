/*
 * test_quiltwire.c — host-side proof that the C codec is byte-identical to
 * the Rust link-core (crates/quilt-wire). Compiled and run with plain gcc
 * (no Arduino, no hardware):
 *
 *   gcc -std=c99 -Wall -Wextra -O2 -o test_quiltwire test_quiltwire.c && ./test_quiltwire
 *
 * Mirrors crates/quilt-wire/tests/roundtrip.rs: golden vector, check value,
 * 20k pseudo-random roundtrips, single-bit corruption rejection.
 */
#include <stdio.h>
#include <stdint.h>
#include "quiltwire.h"

static uint32_t rng_state = 0xC0FFEEu;

static uint32_t rng_u32(void)
{
    uint32_t x = rng_state;
    x ^= x >> 12; x ^= x << 25; x ^= x >> 27;
    rng_state = x;
    return x * 0x2545F491u;
}

static int hexval(char c)
{
    if (c >= '0' && c <= '9') return c - '0';
    if (c >= 'a' && c <= 'f') return c - 'a' + 10;
    return -1;
}

int main(void)
{
    int failures = 0;

    /* CRC check value. */
    uint16_t chk = qw_crc16((const uint8_t *)"123456789", 9);
    if (chk != 0x29B1u) { printf("FAIL: crc check value %04X != 29B1\n", chk); failures++; }

    /* Golden vector — identical bytes to the Rust test. */
    const char *golden_hex = "510101070201e80300000000ac41d6d3";
    uint8_t golden[QW_FRAME_LEN];
    for (uint32_t i = 0; i < QW_FRAME_LEN; i++) {
        golden[i] = (uint8_t)((hexval(golden_hex[i * 2]) << 4) | hexval(golden_hex[i * 2 + 1]));
    }
    qw_frame gf;
    if (qw_decode(golden, &gf) != 0) { printf("FAIL: golden decode\n"); failures++; }
    if (gf.kind != QW_KIND_DELTA || gf.cell_id != 7 || gf.seq != 0x0102u || gf.tick != 1000u) {
        printf("FAIL: golden fields\n"); failures++;
    }
    /* 21.5f == 0x41AC0000 */
    if (gf.value != 21.5f) { printf("FAIL: golden value %f != 21.5\n", (double)gf.value); failures++; }
    uint8_t reenc[QW_FRAME_LEN];
    qw_encode(&gf, reenc);
    if (memcmp(reenc, golden, QW_FRAME_LEN) != 0) { printf("FAIL: golden re-encode\n"); failures++; }

    /* 20k pseudo-random roundtrips. */
    for (uint32_t i = 0; i < 20000u; i++) {
        qw_frame f;
        f.kind = (uint8_t)(rng_u32() % 5);
        f.cell_id = (uint8_t)rng_u32();
        f.seq = (uint16_t)rng_u32();
        f.tick = rng_u32();
        uint32_t bits = rng_u32();
        memcpy(&f.value, &bits, 4);
        uint8_t bytes[QW_FRAME_LEN];
        qw_encode(&f, bytes);
        qw_frame back;
        if (qw_decode(bytes, &back) != 0) { printf("FAIL: roundtrip decode at %u\n", i); failures++; break; }
        if (back.kind != f.kind || back.cell_id != f.cell_id || back.seq != f.seq
            || back.tick != f.tick || memcmp(&back.value, &f.value, 4) != 0) {
            printf("FAIL: roundtrip fields at %u\n", i); failures++; break;
        }
    }

    /* Every single-bit corruption of a valid frame is rejected. */
    qw_frame vf = { QW_KIND_DELTA, 3, 900, 4242, -12.25f };
    uint8_t bytes[QW_FRAME_LEN];
    qw_encode(&vf, bytes);
    for (uint32_t bi = 0; bi < QW_FRAME_LEN; bi++) {
        for (uint32_t bit = 0; bit < 8; bit++) {
            uint8_t bad[QW_FRAME_LEN];
            memcpy(bad, bytes, QW_FRAME_LEN);
            bad[bi] ^= (uint8_t)(1u << bit);
            if (qw_decode(bad, &(qw_frame){0}) >= 0) {
                printf("FAIL: corruption at byte %u bit %u accepted\n", bi, bit);
                failures++;
            }
        }
    }

    if (failures == 0) {
        printf("test_quiltwire: all green (crc check, golden vector, 20000 roundtrips, 128 bit-flips rejected)\n");
        return 0;
    }
    printf("test_quiltwire: %d failure(s)\n", failures);
    return 1;
}
