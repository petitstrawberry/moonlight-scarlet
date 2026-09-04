#pragma once

#include <stddef.h>
#include <stdint.h>

typedef uintptr_t pthread_t;
typedef struct { uintptr_t handle; } pthread_mutex_t;
typedef struct { uintptr_t handle; } pthread_cond_t;
typedef struct { size_t stack_size; } pthread_attr_t;

int pthread_attr_init(pthread_attr_t* attributes);
int pthread_attr_destroy(pthread_attr_t* attributes);
int pthread_attr_setstacksize(pthread_attr_t* attributes, size_t stack_size);
int pthread_create(pthread_t* thread, const pthread_attr_t* attributes,
                   void* (*entry)(void*), void* context);
int pthread_join(pthread_t thread, void** result);
int pthread_detach(pthread_t thread);

int pthread_mutex_init(pthread_mutex_t* mutex, const void* attributes);
int pthread_mutex_destroy(pthread_mutex_t* mutex);
int pthread_mutex_lock(pthread_mutex_t* mutex);
int pthread_mutex_unlock(pthread_mutex_t* mutex);

int pthread_cond_init(pthread_cond_t* cond, const void* attributes);
int pthread_cond_destroy(pthread_cond_t* cond);
int pthread_cond_signal(pthread_cond_t* cond);
int pthread_cond_wait(pthread_cond_t* cond, pthread_mutex_t* mutex);
