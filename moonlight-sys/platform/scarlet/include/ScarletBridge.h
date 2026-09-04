#pragma once

#include <stddef.h>
#include <stdint.h>

typedef void* (*scarlet_thread_entry_t)(void* context);

uint64_t scarlet_monotonic_time_ns(void);
void scarlet_sleep_us(uint64_t microseconds);

uintptr_t scarlet_thread_create(const char* name, scarlet_thread_entry_t entry, void* context);
void scarlet_thread_join(uintptr_t handle);
void scarlet_thread_detach(uintptr_t handle);

uintptr_t scarlet_mutex_create(void);
void scarlet_mutex_destroy(uintptr_t handle);
void scarlet_mutex_lock(uintptr_t handle);
void scarlet_mutex_unlock(uintptr_t handle);

uintptr_t scarlet_cond_create(void);
void scarlet_cond_destroy(uintptr_t handle);
void scarlet_cond_signal(uintptr_t handle);
void scarlet_cond_wait(uintptr_t cond_handle, uintptr_t mutex_handle);

int* scarlet_errno_location(void);
int scarlet_socket_set_nonblocking(int socket, int enabled);
uint32_t scarlet_random_u32(void);
void scarlet_write_bytes(const char* bytes, size_t length);
void scarlet_abort(void) __attribute__((noreturn));
