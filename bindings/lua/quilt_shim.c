/*
 * quilt_shim.c — minimal Lua C module bridging plain Lua 5.1/5.4 to the
 * quilt C ABI (libquilt_cabi.so). Used by quilt_ffi.lua only when LuaJIT's
 * ffi is unavailable; LuaJIT is preferred and needs no shim.
 *
 * The target box has the Lua 5.1 interpreter but no lua headers, so the
 * handful of Lua C API functions used here are declared by hand. The Lua
 * 5.1 ABI is stable for these entry points; symbols resolve from the host
 * interpreter (which exports them, verified via nm -D) at dlopen time.
 *
 * Build (from repo root):
 *   cc -shared -fPIC -O2 -I crates/quilt-cabi \
 *      bindings/lua/quilt_shim.c -o bindings/lua/quilt_shim.so \
 *      -L target/release -lquilt_cabi \
 *      -Wl,-rpath,"$(pwd)/target/release"
 *
 * Memory contract (crates/quilt-cabi/quilt_cabi.h): every char* returned
 * by the library is copied into a Lua string and released with
 * quilt_string_free() before the C function returns. Lua borrows nothing.
 */

#include <stddef.h>
#include <stdint.h>

#include "quilt_cabi.h"

/* ---- hand-declared Lua 5.1 C API (headers unavailable on target) ------- */

typedef struct lua_State lua_State;
typedef long lua_Integer; /* Lua 5.1 default: ptrdiff_t */
typedef int (*lua_CFunction)(lua_State *L);

extern const char  *luaL_checklstring(lua_State *L, int narg, size_t *l);
extern lua_Integer  luaL_checkinteger(lua_State *L, int narg);
extern void         lua_pushstring(lua_State *L, const char *s);
extern void         lua_pushinteger(lua_State *L, lua_Integer n);
extern void         lua_pushnil(lua_State *L);
extern void         lua_pushlightuserdata(lua_State *L, void *p);
extern void        *lua_touserdata(lua_State *L, int index);
extern void         lua_pushcclosure(lua_State *L, lua_CFunction fn, int n);
extern void         lua_setfield(lua_State *L, int index, const char *k);
extern void         lua_createtable(lua_State *L, int narray, int nrec);

/* ---- helpers ------------------------------------------------------------ */

/* Push a library-owned string as a Lua string and release it; nil on NULL. */
static int push_owned(lua_State *L, char *s) {
    if (!s) {
        lua_pushnil(L);
        return 1;
    }
    lua_pushstring(L, s);
    quilt_string_free(s);
    return 1;
}

static QuiltEngine *to_engine(lua_State *L, int i) {
    return (QuiltEngine *)lua_touserdata(L, i); /* NULL tolerated by ABI */
}

/* ---- bindings (thin: one C shim fn per ABI symbol) ---------------------- */

static int l_abi_version(lua_State *L) {
    lua_pushinteger(L, (lua_Integer)quilt_abi_version());
    return 1;
}

static int l_engine_new(lua_State *L) {
    QuiltEngine *e = quilt_engine_new();
    if (!e)
        lua_pushnil(L);
    else
        lua_pushlightuserdata(L, e);
    return 1;
}

static int l_engine_free(lua_State *L) {
    quilt_engine_free(to_engine(L, 1));
    return 0;
}

static int l_engine_load_sheet(lua_State *L) {
    lua_pushinteger(L, quilt_engine_load_sheet(to_engine(L, 1),
                                               luaL_checklstring(L, 2, NULL)));
    return 1;
}

static int l_engine_get(lua_State *L) {
    return push_owned(L, quilt_engine_get(to_engine(L, 1),
                                          luaL_checklstring(L, 2, NULL)));
}

static int l_engine_set(lua_State *L) {
    lua_pushinteger(L, quilt_engine_set(to_engine(L, 1),
                                        luaL_checklstring(L, 2, NULL),
                                        luaL_checklstring(L, 3, NULL)));
    return 1;
}

static int l_ledger_init(lua_State *L) {
    lua_pushinteger(L, quilt_ledger_init(luaL_checklstring(L, 1, NULL),
                                         luaL_checklstring(L, 2, NULL),
                                         (uint64_t)luaL_checkinteger(L, 3)));
    return 1;
}

static int l_ledger_record(lua_State *L) {
    return push_owned(L, quilt_ledger_record(luaL_checklstring(L, 1, NULL),
                                             luaL_checklstring(L, 2, NULL),
                                             luaL_checklstring(L, 3, NULL),
                                             (uint64_t)luaL_checkinteger(L, 4)));
}

static int l_ledger_verify(lua_State *L) {
    lua_pushinteger(L, quilt_ledger_verify(luaL_checklstring(L, 1, NULL)));
    return 1;
}

static int l_ledger_reconcile(lua_State *L) {
    return push_owned(L, quilt_ledger_reconcile(luaL_checklstring(L, 1, NULL)));
}

static int l_ledger_chain_hash(lua_State *L) {
    return push_owned(L, quilt_ledger_chain_hash(luaL_checklstring(L, 1, NULL)));
}

static int l_ledgers_reset(lua_State *L) {
    lua_pushinteger(L, quilt_ledgers_reset());
    return 1;
}

static int l_last_error(lua_State *L) {
    lua_pushstring(L, quilt_last_error()); /* borrowed, never NULL */
    return 1;
}

/* ---- module entry -------------------------------------------------------- */

static const struct {
    const char *name;
    lua_CFunction fn;
} FUNCS[] = {
    {"abi_version", l_abi_version},
    {"engine_new", l_engine_new},
    {"engine_free", l_engine_free},
    {"engine_load_sheet", l_engine_load_sheet},
    {"engine_get", l_engine_get},
    {"engine_set", l_engine_set},
    {"ledger_init", l_ledger_init},
    {"ledger_record", l_ledger_record},
    {"ledger_verify", l_ledger_verify},
    {"ledger_reconcile", l_ledger_reconcile},
    {"ledger_chain_hash", l_ledger_chain_hash},
    {"ledgers_reset", l_ledgers_reset},
    {"last_error", l_last_error},
};

int luaopen_quilt_shim(lua_State *L) {
    size_t i, n = sizeof(FUNCS) / sizeof(FUNCS[0]);
    lua_createtable(L, 0, (int)n);
    for (i = 0; i < n; i++) {
        lua_pushcclosure(L, FUNCS[i].fn, 0);
        lua_setfield(L, -2, FUNCS[i].name);
    }
    return 1;
}
