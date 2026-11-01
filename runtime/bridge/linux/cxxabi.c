#include <stddef.h>

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