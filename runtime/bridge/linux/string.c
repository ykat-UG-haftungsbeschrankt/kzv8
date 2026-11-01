#include <stdint.h>
#include <stddef.h>

void* memcpy(void* destination, const void* source, size_t count) {
    unsigned char* output = (unsigned char*)destination;
    const unsigned char* input = (const unsigned char*)source;
    for (size_t index = 0; index < count; index++) {
        output[index] = input[index];
    }
    return destination;
}

void* memmove(void* destination, const void* source, size_t count) {
    unsigned char* output = (unsigned char*)destination;
    const unsigned char* input = (const unsigned char*)source;
    if ((uintptr_t)output <= (uintptr_t)input ||
        (uintptr_t)output - (uintptr_t)input >= count) {
        for (size_t index = 0; index < count; index++) {
            output[index] = input[index];
        }
    } else {
        for (size_t index = count; index != 0; index--) {
            output[index - 1] = input[index - 1];
        }
    }
    return destination;
}

void* memset(void* destination, int value, size_t count) {
    unsigned char* output = (unsigned char*)destination;
    for (size_t index = 0; index < count; index++) {
        output[index] = (unsigned char)value;
    }
    return destination;
}

int memcmp(const void* left, const void* right, size_t count) {
    const unsigned char* left_bytes = (const unsigned char*)left;
    const unsigned char* right_bytes = (const unsigned char*)right;
    for (size_t index = 0; index < count; index++) {
        if (left_bytes[index] != right_bytes[index]) {
            return (int)left_bytes[index] - (int)right_bytes[index];
        }
    }
    return 0;
}

int bcmp(const void* left, const void* right, size_t count) {
    return memcmp(left, right, count);
}

void* memchr(const void* memory, int value, size_t count) {
    const unsigned char* bytes = (const unsigned char*)memory;
    const unsigned char target = (unsigned char)value;
    for (size_t index = 0; index < count; index++) {
        if (bytes[index] == target) {
            return (void*)(bytes + index);
        }
    }
    return NULL;
}

size_t strlen(const char* text) {
    size_t length = 0;
    while (text[length] != '\0') {
        length++;
    }
    return length;
}

int strcmp(const char* left, const char* right) {
    while (*left != '\0' && *left == *right) {
        left++;
        right++;
    }
    return (unsigned char)*left - (unsigned char)*right;
}

int strncmp(const char* left, const char* right, size_t count) {
    for (size_t index = 0; index < count; index++) {
        const unsigned char left_byte = (unsigned char)left[index];
        const unsigned char right_byte = (unsigned char)right[index];
        if (left_byte != right_byte || left_byte == '\0') {
            return (int)left_byte - (int)right_byte;
        }
    }
    return 0;
}