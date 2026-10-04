#pragma once

#include <stddef.h>

#define EXIT_SUCCESS 0
#define EXIT_FAILURE 1
#define RAND_MAX 2147483647

void* malloc(size_t size);
void* calloc(size_t count, size_t size);
void* aligned_alloc(size_t alignment, size_t size);
void* realloc(void* pointer, size_t size);
void free(void* pointer);
void abort(void) __attribute__((noreturn));

int abs(int value);
int atoi(const char* value);
long strtol(const char* value, char** end, int base);
unsigned long strtoul(const char* value, char** end, int base);
int rand(void);
