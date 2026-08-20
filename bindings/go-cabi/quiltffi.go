package quiltffi

/*
#cgo CFLAGS: -I${SRCDIR}/../../crates/quilt-cabi
#cgo LDFLAGS: -L${SRCDIR}/../../target/release -Wl,-rpath,${SRCDIR}/../../target/release -lquilt_cabi
#include <stdlib.h>
#include "quilt_cabi.h"
*/
import "C"

import (
	"errors"
	"runtime"
	"unsafe"
)

// quilt_last_error is thread-local, so every wrapper pins the goroutine
// to one OS thread for the call AND its error capture; otherwise Go may
// migrate the goroutine between Ms and read another thread's slot.
func lastErrorOr(fallback string) string {
	s := C.GoString(C.quilt_last_error())
	if s == "" {
		return fallback
	}
	return s
}

func AbiVersion() uint32 { return uint32(C.quilt_abi_version()) }

func HeaderAbiVersion() uint32 { return uint32(C.QUILT_ABI_VERSION) }

// LastError is best-effort: it reads the current thread's slot, which is
// only meaningful if the caller is pinned to the thread of the failing
// call (the wrappers capture errors for you, prefer err returns).
func LastError() string {
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	return C.GoString(C.quilt_last_error())
}

type Engine struct {
	ptr *C.QuiltEngine
}

func EngineNew() (*Engine, error) {
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	ptr := C.quilt_engine_new()
	if ptr == nil {
		return nil, errors.New(lastErrorOr("quilt_engine_new: allocation failed"))
	}
	return &Engine{ptr: ptr}, nil
}

func (e *Engine) handle() *C.QuiltEngine {
	if e == nil {
		return nil
	}
	return e.ptr
}

func (e *Engine) Free() { C.quilt_engine_free(e.handle()) }

func (e *Engine) LoadSheet(yaml string) error {
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	cyaml := C.CString(yaml)
	defer C.free(unsafe.Pointer(cyaml))
	if C.quilt_engine_load_sheet(e.handle(), cyaml) != 0 {
		return errors.New(lastErrorOr("quilt_engine_load_sheet failed"))
	}
	return nil
}

func (e *Engine) Get(cellID string) (string, error) {
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	cCell := C.CString(cellID)
	defer C.free(unsafe.Pointer(cCell))
	ret := C.quilt_engine_get(e.handle(), cCell)
	if ret == nil {
		return "", errors.New(lastErrorOr("quilt_engine_get failed"))
	}
	defer C.quilt_string_free(ret)
	return C.GoString(ret), nil
}

func (e *Engine) Set(cellID, valueJSON string) error {
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	cCell := C.CString(cellID)
	cVal := C.CString(valueJSON)
	defer C.free(unsafe.Pointer(cCell))
	defer C.free(unsafe.Pointer(cVal))
	if C.quilt_engine_set(e.handle(), cCell, cVal) != 0 {
		return errors.New(lastErrorOr("quilt_engine_set failed"))
	}
	return nil
}

func LedgerInit(cellID, genesisJSON string, tsMillis uint64) error {
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	cCell := C.CString(cellID)
	cGenesis := C.CString(genesisJSON)
	defer C.free(unsafe.Pointer(cCell))
	defer C.free(unsafe.Pointer(cGenesis))
	if C.quilt_ledger_init(cCell, cGenesis, C.uint64_t(tsMillis)) != 0 {
		return errors.New(lastErrorOr("quilt_ledger_init failed"))
	}
	return nil
}

func LedgerRecord(cellID, inputJSON, outputJSON string, tsMillis uint64) (string, error) {
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	cCell := C.CString(cellID)
	cIn := C.CString(inputJSON)
	cOut := C.CString(outputJSON)
	defer C.free(unsafe.Pointer(cCell))
	defer C.free(unsafe.Pointer(cIn))
	defer C.free(unsafe.Pointer(cOut))
	ret := C.quilt_ledger_record(cCell, cIn, cOut, C.uint64_t(tsMillis))
	if ret == nil {
		return "", errors.New(lastErrorOr("quilt_ledger_record failed"))
	}
	defer C.quilt_string_free(ret)
	return C.GoString(ret), nil
}

func LedgerVerify(cellID string) (bool, error) {
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	cCell := C.CString(cellID)
	defer C.free(unsafe.Pointer(cCell))
	switch C.quilt_ledger_verify(cCell) {
	case 1:
		return true, nil
	case 0:
		return false, nil
	default:
		return false, errors.New(lastErrorOr("quilt_ledger_verify: no such ledger"))
	}
}

func LedgerReconcile(cellID string) (string, error) {
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	cCell := C.CString(cellID)
	defer C.free(unsafe.Pointer(cCell))
	ret := C.quilt_ledger_reconcile(cCell)
	if ret == nil {
		return "", errors.New(lastErrorOr("quilt_ledger_reconcile failed"))
	}
	defer C.quilt_string_free(ret)
	return C.GoString(ret), nil
}

func LedgerChainHash(cellID string) (string, error) {
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	cCell := C.CString(cellID)
	defer C.free(unsafe.Pointer(cCell))
	ret := C.quilt_ledger_chain_hash(cCell)
	if ret == nil {
		return "", errors.New(lastErrorOr("quilt_ledger_chain_hash failed"))
	}
	defer C.quilt_string_free(ret)
	return C.GoString(ret), nil
}

func LedgersReset() { C.quilt_ledgers_reset() }
