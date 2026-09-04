#pragma once

#include <stdarg.h>
#include <stddef.h>

typedef struct ScarletFile FILE;

#define EOF (-1)
#define SEEK_SET 0
#define SEEK_CUR 1
#define SEEK_END 2

int snprintf(char* buffer, size_t length, const char* format, ...);
int vsnprintf(char* buffer, size_t length, const char* format, va_list arguments);
int printf(const char* format, ...);
void perror(const char* message);

FILE* fopen(const char* path, const char* mode);
int fclose(FILE* file);
size_t fread(void* buffer, size_t size, size_t count, FILE* file);
size_t fwrite(const void* buffer, size_t size, size_t count, FILE* file);
int fseek(FILE* file, long offset, int origin);
long ftell(FILE* file);
