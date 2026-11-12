// The caller: plain COM through a hand-declared vtable, as any C or C++
// consumer of the ABI would write it. Three loops over the same object:
//   vtable   p->vtbl->Add(p, ...)       -- what a projection generates
//   direct   probe_adder_add(p, ...)    -- the exported symbol, upper bound
// The salt and the iteration count come from argv so nothing folds at
// compile time; the accumulator feeds each call so the loop cannot be
// vectorized away.
#include <chrono>
#include <cstdint>
#include <cstdio>
#include <cstdlib>

extern "C" __declspec(dllimport) void* __stdcall GetModuleHandleA(const char*);

struct IAdder;
struct IAdderVtbl {
    long (__stdcall *QueryInterface)(IAdder*, const void*, void**);
    unsigned long (__stdcall *AddRef)(IAdder*);
    unsigned long (__stdcall *Release)(IAdder*);
    long (__stdcall *Add)(IAdder*, int, int, int*);
};
struct IAdder { const IAdderVtbl* vtbl; };

extern "C" void* __stdcall probe_create_adder(int salt);
extern "C" long __stdcall probe_adder_add(void* self, int a, int b, int* out);

static double seconds_since(std::chrono::steady_clock::time_point t0) {
    return std::chrono::duration<double>(std::chrono::steady_clock::now() - t0).count();
}

// Separate functions so each loop is one symbol to disassemble.
__attribute__((noinline)) int run_vtable(IAdder* p, int n) {
    int acc = 0;
    for (int i = 0; i < n; i++) {
        int out;
        p->vtbl->Add(p, i, acc, &out);
        acc = out;
    }
    return acc;
}

__attribute__((noinline)) int run_direct(IAdder* p, int n) {
    int acc = 0;
    for (int i = 0; i < n; i++) {
        int out;
        probe_adder_add(p, i, acc, &out);
        acc = out;
    }
    return acc;
}

// The object is made and used in one function: if the optimizer can see
// through the (bitcode) constructor to the vtable it stored, this is where
// a vtable call could become direct.
__attribute__((noinline)) int run_vtable_fresh(int salt, int n) {
    auto* p = static_cast<IAdder*>(probe_create_adder(salt));
    int acc = 0;
    for (int i = 0; i < n; i++) {
        int out;
        p->vtbl->Add(p, i, acc, &out);
        acc = out;
    }
    p->vtbl->Release(p);
    return acc;
}

// As above, but the slot is read once, right after construction, where no
// opaque call has happened since the constructor stored the vtable: the
// value-flow proof the optimizer accepts, if it accepts any.
__attribute__((noinline)) int run_vtable_hoisted(int salt, int n) {
    auto* p = static_cast<IAdder*>(probe_create_adder(salt));
    auto add = p->vtbl->Add;
    auto release = p->vtbl->Release;
    int acc = 0;
    for (int i = 0; i < n; i++) {
        int out;
        add(p, i, acc, &out);
        acc = out;
    }
    release(p);
    return acc;
}

int main(int argc, char** argv) {
    int salt = argc > 1 ? std::atoi(argv[1]) : 7;
    int n = argc > 2 ? std::atoi(argv[2]) : 200'000'000;
    auto* p = static_cast<IAdder*>(probe_create_adder(salt));

    auto t0 = std::chrono::steady_clock::now();
    int a = run_vtable(p, n);
    double tv = seconds_since(t0);

    t0 = std::chrono::steady_clock::now();
    int b = run_direct(p, n);
    double td = seconds_since(t0);

    p->vtbl->Release(p);

    t0 = std::chrono::steady_clock::now();
    int c = run_vtable_fresh(salt, n);
    double tf = seconds_since(t0);

    t0 = std::chrono::steady_clock::now();
    int d = run_vtable_hoisted(salt, n);
    double th = seconds_since(t0);

    std::printf("vtable %.3f ns/call  direct %.3f ns/call  fresh %.3f ns/call  hoisted %.3f ns/call  (acc %d %d %d %d)\n",
                tv * 1e9 / n, td * 1e9 / n, tf * 1e9 / n, th * 1e9 / n, a, b, c, d);
    // Where the loops are, for build.sh to disassemble: the stripped image
    // has no symbol table, so RVAs are the only handle.
    auto base = reinterpret_cast<std::uintptr_t>(GetModuleHandleA(nullptr));
    std::printf("rva run_vtable=%llx run_direct=%llx run_vtable_fresh=%llx run_vtable_hoisted=%llx\n",
                (unsigned long long)(reinterpret_cast<std::uintptr_t>(&run_vtable) - base),
                (unsigned long long)(reinterpret_cast<std::uintptr_t>(&run_direct) - base),
                (unsigned long long)(reinterpret_cast<std::uintptr_t>(&run_vtable_fresh) - base),
                (unsigned long long)(reinterpret_cast<std::uintptr_t>(&run_vtable_hoisted) - base));
    return a == b && b == c && c == d ? 0 : 1;
}
