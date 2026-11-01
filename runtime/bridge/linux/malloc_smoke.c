#include <stdint.h>
#include <stdlib.h>

int kz_c_malloc_smoke(void) {
    unsigned char *first = malloc(37);
    unsigned char *second = malloc(91);
    if (first == 0 || second == 0 || first == second
        || (uintptr_t)first % 16 != 0 || (uintptr_t)second % 16 != 0) {
        return -1;
    }
    first[0] = 0x5a;
    first[36] = 0xa5;
    second[0] = 0x3c;

    free(first);
    unsigned char *reused = malloc(37);
    if (reused != first) {
        return -1;
    }
    reused[0] = 0x11;
    free(reused);
    free(second);
    free(0);

    void *coalesced = malloc(8192);
    if (coalesced == 0) {
        return -1;
    }
    free(coalesced);
    return 0;
}