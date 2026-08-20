/*
 * smoke.c — quilt-cabi smoke test: link the real cdylib, exercise the ABI
 * against compat/golden.json values.
 *
 * Covers: value read (op a), formula eval + reactive propagation (op b),
 * ledger record/verify/reconcile with bit-for-bit seals (op e), plus the
 * error discipline. All golden constants come from golden_vectors.h,
 * generated from compat/golden.json by gen-sheet.py (never hand-copied).
 *
 * Build + run via smoke/run.sh (or see docs/c-abi.md):
 *
 *   cc -std=c11 -Wall -Wextra -I.. smoke.c -o smoke \
 *      -L$TARGET_DIR -lquilt_cabi -Wl,-rpath,$TARGET_DIR && ./smoke
 */

#include <stdio.h>
#include <string.h>

#include "quilt_cabi.h"
#include "golden_vectors.h"

static int failures = 0;
static int checks = 0;

#define CHECK(cond, msg)                                                   \
    do {                                                                   \
        checks++;                                                          \
        if (cond) {                                                        \
            printf("  PASS %s\n", msg);                                    \
        } else {                                                           \
            failures++;                                                    \
            printf("  FAIL %s\n", msg);                                    \
        }                                                                  \
    } while (0)

/* Read a cell and compare its JSON text exactly. */
static int get_is(QuiltEngine *e, const char *cell, const char *want) {
    char *got = quilt_engine_get(e, cell);
    if (!got) {
        printf("    get(%s) returned NULL: %s\n", cell, quilt_last_error());
        return 0;
    }
    int ok = strcmp(got, want) == 0;
    if (!ok)
        printf("    get(%s): got \"%s\", want \"%s\"\n", cell, got, want);
    quilt_string_free(got);
    return ok;
}

/* Record one golden transaction and return the seal (caller frees). */
static char *record(const char *cell, const char *in, const char *out,
                    uint64_t ts) {
    char *seal = quilt_ledger_record(cell, in, out, ts);
    if (!seal)
        printf("    ledger_record failed: %s\n", quilt_last_error());
    return seal;
}

int main(void) {
    printf("=== quilt-cabi C smoke (linked against libquilt_cabi.so) ===\n");
    CHECK(quilt_abi_version() == QUILT_ABI_VERSION,
          "ABI version matches quilt_cabi.h");

    quilt_ledgers_reset();

    /* -- engine + golden sheet ------------------------------------------- */

    QuiltEngine *e = quilt_engine_new();
    CHECK(e != NULL, "engine_new");
    CHECK(quilt_engine_load_sheet(e, SHEET_YAML) == 0, "load_sheet (golden YAML)");

    /* -- op (a): value cell read — exact JSON equality -------------------- */

    CHECK(get_is(e, "bilge.threshold", "80.0"), "(a) read bilge.threshold == 80.0");
    CHECK(get_is(e, "status", "\"idle\""), "(a) read status == \"idle\"");
    CHECK(get_is(e, "bilge.level", "40.0"), "(a) read bilge.level == 40.0");

    /* -- op (b): formula eval, initial + post-push ------------------------- */

    CHECK(get_is(e, "pump.should_run", "false"), "(b) initial should_run == false");
    CHECK(get_is(e, "pump.relay_cmd", "-20.0"), "(b) initial relay_cmd == -20.0");
    CHECK(quilt_engine_set(e, "bilge.level", "85.0") == 0, "(b) push level=85.0");
    CHECK(get_is(e, "bilge.level", "85.0"), "(b) post level == 85.0");
    CHECK(get_is(e, "pump.should_run", "true"), "(b) post should_run == true");
    CHECK(get_is(e, "pump.relay_cmd", "2.5"), "(b) post relay_cmd == 2.5");

    /* -- op (e): ledger record / verify / reconcile, seals bit-for-bit ----- */

    CHECK(quilt_ledger_init("bilge.level", "40.0", 1000) == 0,
          "(e) ledger_init genesis 40.0 @1000");
    CHECK(quilt_ledger_init("bilge.level", "40.0", 1000) == -1,
          "(e) double ledger_init is rejected");

    /* The empty ledger's chain hash is the genesis commit — the golden
     * root that entry 1's prev-link seals against. */
    char *root = quilt_ledger_chain_hash("bilge.level");
    CHECK(root && strcmp(root, G_ENTRY1_PREV) == 0, "(e) genesis root pinned");
    quilt_string_free(root);

    char *s1 = record("bilge.level", "85.0", "85.0", 2000);
    char *s2 = record("bilge.level", "87.5", "87.5", 3000);
    char *s3 = record("bilge.level", "87.5", "87.5", 4000);
    CHECK(s1 && strcmp(s1, G_ENTRY1) == 0, "(e) seal 1 bit-for-bit");
    CHECK(s2 && strcmp(s2, G_ENTRY2) == 0, "(e) seal 2 bit-for-bit");
    CHECK(s3 && strcmp(s3, G_ENTRY3) == 0, "(e) seal 3 bit-for-bit");

    CHECK(quilt_ledger_verify("bilge.level") == 1, "(e) chain verifies (1)");
    CHECK(quilt_ledger_verify("no.such.cell") == -1, "(e) unknown ledger -> -1");

    char *head = quilt_ledger_chain_hash("bilge.level");
    CHECK(head && strcmp(head, G_CHAIN_HASH) == 0, "(e) chain_hash == golden head");
    quilt_string_free(head);

    char *report = quilt_ledger_reconcile("bilge.level");
    int rec_ok = report != NULL;
    for (int i = 0; rec_ok && G_RECONCILE_NEEDLES[i]; i++)
        if (!strstr(report, G_RECONCILE_NEEDLES[i])) {
            printf("    reconcile missing %s\n    got: %s\n",
                   G_RECONCILE_NEEDLES[i], report);
            rec_ok = 0;
        }
    CHECK(rec_ok, "(e) reconcile report matches golden");
    quilt_string_free(report);
    quilt_string_free(s1);
    quilt_string_free(s2);
    quilt_string_free(s3);

    /* -- error discipline --------------------------------------------------- */

    CHECK(quilt_engine_get(e, "no.such.cell") == NULL,
          "unknown cell returns NULL");
    CHECK(strlen(quilt_last_error()) > 0, "last_error explains the failure");
    CHECK(quilt_engine_get(NULL, "x") == NULL, "NULL engine tolerated");
    CHECK(quilt_ledger_record("x.cell", "{not json", "1", 1) == NULL,
          "bad JSON input returns NULL");

    quilt_engine_free(e);
    quilt_string_free(NULL); /* must be a no-op */

    printf("RESULT: %s — %d checks, %d failures\n",
           failures == 0 ? "PASS" : "FAIL", checks, failures);
    return failures == 0 ? 0 : 1;
}
