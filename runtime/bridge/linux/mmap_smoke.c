#include <errno.h>
#include <sys/mman.h>

int kz_c_mmap_smoke(void) {
    const size_t length = 2 * 4096;
    volatile unsigned char *memory = mmap(0, length, PROT_READ | PROT_WRITE,
                                          MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (memory == MAP_FAILED || (uintptr_t)memory % 4096 != 0) {
        return -1;
    }
    if (memory[0] != 0 || memory[length - 1] != 0) {
        return -1;
    }
    memory[0] = 0x5a;
    memory[length - 1] = 0xa5;
    if (munmap((void *)memory, length) != 0) {
        return -1;
    }

    memory = mmap(0, length, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (memory == MAP_FAILED || memory[0] != 0 || memory[length - 1] != 0) {
        return -1;
    }
    if (munmap((void *)memory, length) != 0) {
        return -1;
    }

    errno = 0;
    if (mmap(0, 0, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0) != MAP_FAILED
        || errno != EINVAL) {
        return -1;
    }
    return 0;
}