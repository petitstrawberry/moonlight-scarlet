#include <ScarletBridge.h>
#include <errno.h>
#include <fcntl.h>
#include <pthread.h>
#include <signal.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/select.h>
#include <sys/time.h>
#include <time.h>
#include <unistd.h>

int pthread_attr_init(pthread_attr_t* attributes) {
    attributes->stack_size = 0;
    return 0;
}

int pthread_attr_destroy(pthread_attr_t* attributes) {
    (void)attributes;
    return 0;
}

int pthread_attr_setstacksize(pthread_attr_t* attributes, size_t stack_size) {
    attributes->stack_size = stack_size;
    return 0;
}

int pthread_create(pthread_t* thread, const pthread_attr_t* attributes,
                   void* (*entry)(void*), void* context) {
    (void)attributes;
    *thread = scarlet_thread_create(NULL, entry, context);
    return *thread == 0 ? EAGAIN : 0;
}

int pthread_join(pthread_t thread, void** result) {
    scarlet_thread_join(thread);
    if (result != NULL) {
        *result = NULL;
    }
    return 0;
}

int pthread_detach(pthread_t thread) {
    scarlet_thread_detach(thread);
    return 0;
}

int pthread_mutex_init(pthread_mutex_t* mutex, const void* attributes) {
    (void)attributes;
    mutex->handle = scarlet_mutex_create();
    return mutex->handle == 0 ? ENOMEM : 0;
}

int pthread_mutex_destroy(pthread_mutex_t* mutex) {
    scarlet_mutex_destroy(mutex->handle);
    mutex->handle = 0;
    return 0;
}

int pthread_mutex_lock(pthread_mutex_t* mutex) {
    scarlet_mutex_lock(mutex->handle);
    return 0;
}

int pthread_mutex_unlock(pthread_mutex_t* mutex) {
    scarlet_mutex_unlock(mutex->handle);
    return 0;
}

int pthread_cond_init(pthread_cond_t* cond, const void* attributes) {
    (void)attributes;
    cond->handle = scarlet_cond_create();
    return cond->handle == 0 ? ENOMEM : 0;
}

int pthread_cond_destroy(pthread_cond_t* cond) {
    scarlet_cond_destroy(cond->handle);
    cond->handle = 0;
    return 0;
}

int pthread_cond_signal(pthread_cond_t* cond) {
    scarlet_cond_signal(cond->handle);
    return 0;
}

int pthread_cond_wait(pthread_cond_t* cond, pthread_mutex_t* mutex) {
    scarlet_cond_wait(cond->handle, mutex->handle);
    return 0;
}

int usleep(useconds_t microseconds) {
    scarlet_sleep_us(microseconds);
    return 0;
}

int gettimeofday(struct timeval* value, void* timezone) {
    (void)timezone;
    uint64_t now = scarlet_monotonic_time_ns();
    value->tv_sec = (time_t)(now / 1000000000ULL);
    value->tv_usec = (long)((now % 1000000000ULL) / 1000ULL);
    return 0;
}

int clock_gettime(int clock_id, struct timespec* value) {
    (void)clock_id;
    uint64_t now = scarlet_monotonic_time_ns();
    value->tv_sec = (time_t)(now / 1000000000ULL);
    value->tv_nsec = (long)(now % 1000000000ULL);
    return 0;
}

time_t time(time_t* result) {
    time_t seconds = (time_t)(scarlet_monotonic_time_ns() / 1000000000ULL);
    if (result != NULL) {
        *result = seconds;
    }
    return seconds;
}

int sigemptyset(sigset_t* set) {
    *set = 0;
    return 0;
}

int sigaction(int signal, const struct sigaction* action, struct sigaction* previous) {
    (void)signal;
    (void)action;
    (void)previous;
    return 0;
}

int ioctl(int descriptor, unsigned long request, ...) {
    if (request != FIONBIO) {
        errno = EOPNOTSUPP;
        return -1;
    }

    va_list arguments;
    va_start(arguments, request);
    int* enabled = va_arg(arguments, int*);
    va_end(arguments);
    return scarlet_socket_set_nonblocking(descriptor, *enabled != 0);
}

int fcntl(int descriptor, int command, ...) {
    (void)descriptor;
    (void)command;
    errno = EOPNOTSUPP;
    return -1;
}

int select(int count, fd_set* read_set, fd_set* write_set, fd_set* error_set,
           struct timeval* timeout) {
    (void)count;
    (void)read_set;
    (void)write_set;
    (void)error_set;
    (void)timeout;
    errno = EOPNOTSUPP;
    return -1;
}

void* memcpy(void* destination, const void* source, size_t length) {
    unsigned char* out = destination;
    const unsigned char* in = source;
    for (size_t index = 0; index < length; index++) {
        out[index] = in[index];
    }
    return destination;
}

void* memmove(void* destination, const void* source, size_t length) {
    unsigned char* out = destination;
    const unsigned char* in = source;
    if (out < in) {
        for (size_t index = 0; index < length; index++) {
            out[index] = in[index];
        }
    }
    else if (out > in) {
        while (length > 0) {
            length--;
            out[length] = in[length];
        }
    }
    return destination;
}

void* memset(void* destination, int value, size_t length) {
    unsigned char* out = destination;
    for (size_t index = 0; index < length; index++) {
        out[index] = (unsigned char)value;
    }
    return destination;
}

int memcmp(const void* left, const void* right, size_t length) {
    const unsigned char* a = left;
    const unsigned char* b = right;
    for (size_t index = 0; index < length; index++) {
        if (a[index] != b[index]) {
            return (int)a[index] - (int)b[index];
        }
    }
    return 0;
}

void* memchr(const void* bytes, int value, size_t length) {
    const unsigned char* input = bytes;
    for (size_t index = 0; index < length; index++) {
        if (input[index] == (unsigned char)value) {
            return (void*)&input[index];
        }
    }
    return NULL;
}

size_t strlen(const char* string) {
    const char* end = string;
    while (*end != '\0') {
        end++;
    }
    return (size_t)(end - string);
}

char* strcpy(char* destination, const char* source) {
    char* result = destination;
    while ((*destination++ = *source++) != '\0') {}
    return result;
}

char* strncpy(char* destination, const char* source, size_t length) {
    size_t index = 0;
    while (index < length && source[index] != '\0') {
        destination[index] = source[index];
        index++;
    }
    while (index < length) {
        destination[index++] = '\0';
    }
    return destination;
}

int strcmp(const char* left, const char* right) {
    while (*left != '\0' && *left == *right) {
        left++;
        right++;
    }
    return (unsigned char)*left - (unsigned char)*right;
}

int strncmp(const char* left, const char* right, size_t length) {
    for (size_t index = 0; index < length; index++) {
        unsigned char a = (unsigned char)left[index];
        unsigned char b = (unsigned char)right[index];
        if (a != b || a == '\0') {
            return (int)a - (int)b;
        }
    }
    return 0;
}

char* strchr(const char* string, int character) {
    for (;; string++) {
        if (*string == (char)character) {
            return (char*)string;
        }
        if (*string == '\0') {
            return NULL;
        }
    }
}

char* strrchr(const char* string, int character) {
    const char* found = NULL;
    do {
        if (*string == (char)character) {
            found = string;
        }
    } while (*string++ != '\0');
    return (char*)found;
}

char* strstr(const char* haystack, const char* needle) {
    size_t needle_length = strlen(needle);
    if (needle_length == 0) {
        return (char*)haystack;
    }
    for (; *haystack != '\0'; haystack++) {
        if (strncmp(haystack, needle, needle_length) == 0) {
            return (char*)haystack;
        }
    }
    return NULL;
}

static int is_delimiter(char character, const char* delimiters) {
    return strchr(delimiters, character) != NULL;
}

char* strtok_r(char* string, const char* delimiters, char** state) {
    char* cursor = string != NULL ? string : *state;
    while (*cursor != '\0' && is_delimiter(*cursor, delimiters)) {
        cursor++;
    }
    if (*cursor == '\0') {
        *state = cursor;
        return NULL;
    }

    char* token = cursor;
    while (*cursor != '\0' && !is_delimiter(*cursor, delimiters)) {
        cursor++;
    }
    if (*cursor != '\0') {
        *cursor++ = '\0';
    }
    *state = cursor;
    return token;
}

char* strdup(const char* string) {
    size_t length = strlen(string) + 1;
    char* copy = malloc(length);
    if (copy != NULL) {
        memcpy(copy, string, length);
    }
    return copy;
}

static int digit_value(char character) {
    if (character >= '0' && character <= '9') {
        return character - '0';
    }
    if (character >= 'a' && character <= 'z') {
        return character - 'a' + 10;
    }
    if (character >= 'A' && character <= 'Z') {
        return character - 'A' + 10;
    }
    return -1;
}

static const char* skip_space(const char* value) {
    while (*value == ' ' || *value == '\t' || *value == '\n' ||
           *value == '\r' || *value == '\f' || *value == '\v') {
        value++;
    }
    return value;
}

unsigned long strtoul(const char* value, char** end, int base) {
    const char* start = value;
    value = skip_space(value);
    int negative = 0;
    if (*value == '+' || *value == '-') {
        negative = *value == '-';
        value++;
    }
    if ((base == 0 || base == 16) && value[0] == '0' &&
        (value[1] == 'x' || value[1] == 'X')) {
        base = 16;
        value += 2;
    }
    else if (base == 0) {
        base = value[0] == '0' ? 8 : 10;
    }

    const char* digits = value;
    unsigned long result = 0;
    int digit;
    while ((digit = digit_value(*value)) >= 0 && digit < base) {
        result = result * (unsigned long)base + (unsigned long)digit;
        value++;
    }
    if (end != NULL) {
        *end = (char*)(value == digits ? start : value);
    }
    return negative ? (unsigned long)(0 - result) : result;
}

long strtol(const char* value, char** end, int base) {
    const char* cursor = skip_space(value);
    int negative = *cursor == '-';
    if (*cursor == '+' || *cursor == '-') {
        cursor++;
    }
    char* parsed_end;
    unsigned long magnitude = strtoul(cursor, &parsed_end, base);
    if (end != NULL) {
        *end = parsed_end == cursor ? (char*)value : parsed_end;
    }
    return negative ? -(long)magnitude : (long)magnitude;
}

int atoi(const char* value) {
    return (int)strtol(value, NULL, 10);
}

int abs(int value) {
    return value < 0 ? -value : value;
}

int rand(void) {
    return (int)(scarlet_random_u32() & RAND_MAX);
}

struct output_buffer {
    char* bytes;
    size_t capacity;
    size_t position;
    size_t total;
};

static void output_character(struct output_buffer* output, char character) {
    if (output->capacity > 0 && output->position + 1 < output->capacity) {
        output->bytes[output->position] = character;
    }
    output->position++;
    output->total++;
}

static void output_repeat(struct output_buffer* output, char character, int count) {
    while (count-- > 0) {
        output_character(output, character);
    }
}

static void output_string(struct output_buffer* output, const char* string, int maximum) {
    int count = 0;
    while (*string != '\0' && (maximum < 0 || count < maximum)) {
        output_character(output, *string++);
        count++;
    }
}

static int unsigned_length(unsigned long long value, unsigned base) {
    int length = 1;
    while (value >= base) {
        value /= base;
        length++;
    }
    return length;
}

static void output_unsigned(struct output_buffer* output, unsigned long long value,
                            unsigned base, int uppercase, int width, int zero_pad) {
    char digits[32];
    const char* alphabet = uppercase ? "0123456789ABCDEF" : "0123456789abcdef";
    int length = unsigned_length(value, base);
    int index = length;
    while (index > 0) {
        digits[--index] = alphabet[value % base];
        value /= base;
    }
    output_repeat(output, zero_pad ? '0' : ' ', width - length);
    for (index = 0; index < length; index++) {
        output_character(output, digits[index]);
    }
}

int vsnprintf(char* buffer, size_t length, const char* format, va_list arguments) {
    struct output_buffer output = { buffer, length, 0, 0 };
    va_list args;
    va_copy(args, arguments);

    while (*format != '\0') {
        if (*format++ != '%') {
            output_character(&output, format[-1]);
            continue;
        }
        if (*format == '%') {
            output_character(&output, *format++);
            continue;
        }

        int zero_pad = 0;
        int left_align = 0;
        int plus = 0;
        for (;;) {
            if (*format == '0') zero_pad = 1;
            else if (*format == '-') left_align = 1;
            else if (*format == '+') plus = 1;
            else if (*format == ' ' || *format == '#') {}
            else break;
            format++;
        }

        int width = 0;
        if (*format == '*') {
            width = va_arg(args, int);
            format++;
        }
        else {
            while (*format >= '0' && *format <= '9') {
                width = width * 10 + (*format++ - '0');
            }
        }

        int precision = -1;
        if (*format == '.') {
            format++;
            precision = 0;
            if (*format == '*') {
                precision = va_arg(args, int);
                format++;
            }
            else {
                while (*format >= '0' && *format <= '9') {
                    precision = precision * 10 + (*format++ - '0');
                }
            }
        }

        int length_modifier = 0;
        if (*format == 'l') {
            length_modifier = 1;
            format++;
            if (*format == 'l') {
                length_modifier = 2;
                format++;
            }
        }
        else if (*format == 'z' || *format == 't' || *format == 'j') {
            length_modifier = 2;
            format++;
        }
        else if (*format == 'h') {
            format++;
            if (*format == 'h') format++;
        }

        char conversion = *format == '\0' ? '\0' : *format++;
        if (conversion == 's') {
            const char* string = va_arg(args, const char*);
            if (string == NULL) string = "(null)";
            int text_length = (int)strlen(string);
            if (precision >= 0 && text_length > precision) text_length = precision;
            if (!left_align) output_repeat(&output, ' ', width - text_length);
            output_string(&output, string, text_length);
            if (left_align) output_repeat(&output, ' ', width - text_length);
        }
        else if (conversion == 'c') {
            output_character(&output, (char)va_arg(args, int));
        }
        else if (conversion == 'd' || conversion == 'i') {
            long long signed_value = length_modifier == 2 ? va_arg(args, long long) :
                                     length_modifier == 1 ? va_arg(args, long) : va_arg(args, int);
            int negative = signed_value < 0;
            unsigned long long magnitude = negative ?
                (unsigned long long)(-(signed_value + 1)) + 1 : (unsigned long long)signed_value;
            int sign_length = negative || plus;
            int number_length = unsigned_length(magnitude, 10);
            if (!zero_pad) output_repeat(&output, ' ', width - number_length - sign_length);
            if (negative) output_character(&output, '-');
            else if (plus) output_character(&output, '+');
            output_unsigned(&output, magnitude, 10, 0,
                            zero_pad ? width - sign_length : number_length, zero_pad);
        }
        else if (conversion == 'u' || conversion == 'x' || conversion == 'X' || conversion == 'o') {
            unsigned long long value = length_modifier == 2 ? va_arg(args, unsigned long long) :
                                       length_modifier == 1 ? va_arg(args, unsigned long) : va_arg(args, unsigned int);
            unsigned base = conversion == 'o' ? 8 : (conversion == 'u' ? 10 : 16);
            output_unsigned(&output, value, base, conversion == 'X', width, zero_pad);
        }
        else if (conversion == 'p') {
            uintptr_t value = (uintptr_t)va_arg(args, void*);
            output_string(&output, "0x", -1);
            output_unsigned(&output, value, 16, 0, (int)(sizeof(void*) * 2), 1);
        }
        else if (conversion == 'f' || conversion == 'F' || conversion == 'e' || conversion == 'g') {
            (void)va_arg(args, double);
            output_string(&output, "<float>", -1);
        }
        else if (conversion != '\0') {
            output_character(&output, '%');
            output_character(&output, conversion);
        }
    }

    va_end(args);
    if (length > 0) {
        size_t terminator = output.position < length ? output.position : length - 1;
        buffer[terminator] = '\0';
    }
    return (int)output.total;
}

int snprintf(char* buffer, size_t length, const char* format, ...) {
    va_list arguments;
    va_start(arguments, format);
    int result = vsnprintf(buffer, length, format, arguments);
    va_end(arguments);
    return result;
}

int printf(const char* format, ...) {
    char buffer[1024];
    va_list arguments;
    va_start(arguments, format);
    int result = vsnprintf(buffer, sizeof(buffer), format, arguments);
    va_end(arguments);
    size_t written = result < 0 ? 0 : (size_t)result;
    if (written >= sizeof(buffer)) written = sizeof(buffer) - 1;
    scarlet_write_bytes(buffer, written);
    return result;
}

void perror(const char* message) {
    if (message != NULL && *message != '\0') {
        scarlet_write_bytes(message, strlen(message));
        scarlet_write_bytes(": ", 2);
    }
    scarlet_write_bytes("Scarlet system error\n", 21);
}

void abort(void) {
    scarlet_abort();
}
