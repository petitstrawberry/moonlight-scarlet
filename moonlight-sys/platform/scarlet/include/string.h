#pragma once

#include <stddef.h>

void* memcpy(void* destination, const void* source, size_t length);
void* memmove(void* destination, const void* source, size_t length);
void* memset(void* destination, int value, size_t length);
int memcmp(const void* left, const void* right, size_t length);
void* memchr(const void* bytes, int value, size_t length);

size_t strlen(const char* string);
char* strcpy(char* destination, const char* source);
char* strncpy(char* destination, const char* source, size_t length);
int strcmp(const char* left, const char* right);
int strncmp(const char* left, const char* right, size_t length);
char* strchr(const char* string, int character);
char* strrchr(const char* string, int character);
char* strstr(const char* haystack, const char* needle);
char* strtok_r(char* string, const char* delimiters, char** state);
char* strdup(const char* string);
