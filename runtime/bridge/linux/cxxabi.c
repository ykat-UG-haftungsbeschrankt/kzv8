#include <stddef.h>
#include <stdint.h>

#define MAX_EXIT_HANDLERS 256

typedef void (*Destructor)(void* object);

struct ExitHandler {
    Destructor destructor;
    void* object;
    void* dso;
    int active;
};

static struct ExitHandler handlers[MAX_EXIT_HANDLERS];
static size_t handler_count;

int __cxa_atexit(Destructor destructor, void* object, void* dso) {
    if (destructor == NULL || handler_count == MAX_EXIT_HANDLERS) return -1;
    handlers[handler_count++] = (struct ExitHandler){destructor, object, dso, 1};
    return 0;
}

void __cxa_finalize(void* dso) {
    for (size_t index = handler_count; index != 0; index--) {
        struct ExitHandler* handler = &handlers[index - 1];
        if (!handler->active || (dso != NULL && handler->dso != dso)) continue;
        handler->active = 0;
        handler->destructor(handler->object);
    }
}

typedef int UnwindReasonCode;
typedef uintptr_t UnwindWord;
typedef struct UnwindContext UnwindContext;
typedef struct UnwindException UnwindException;

UnwindWord _Unwind_GetRegionStart(UnwindContext* context) {
    (void)context;
    return 0;
}

void* _Unwind_GetLanguageSpecificData(UnwindContext* context) {
    (void)context;
    return NULL;
}

void _Unwind_SetIP(UnwindContext* context, UnwindWord instruction_pointer) {
    (void)context;
    (void)instruction_pointer;
}

UnwindReasonCode _Unwind_RaiseException(UnwindException* exception) {
    (void)exception;
    return 3;
}

void _Unwind_Resume(UnwindException* exception) {
    (void)exception;
    __builtin_trap();
}

void _Unwind_DeleteException(UnwindException* exception) {
    (void)exception;
}