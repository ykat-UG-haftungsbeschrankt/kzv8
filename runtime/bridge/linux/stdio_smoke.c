#include <stdio.h>

int kz_c_stdio_smoke(void) {
    if (printf("kz: C stdio printf %d %s 0x%x\n", 42, "hello", 42) < 0) {
        return EOF;
    }
    if (puts("kz: C stdio puts works") == EOF) {
        return EOF;
    }
    if (putchar('!') == EOF || putchar('\n') == EOF) {
        return EOF;
    }
    return 0;
}