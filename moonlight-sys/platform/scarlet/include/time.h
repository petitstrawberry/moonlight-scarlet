#pragma once

#include <sys/types.h>

typedef long clock_t;

struct timespec {
    time_t tv_sec;
    long tv_nsec;
};

#define CLOCK_REALTIME 0
#define CLOCK_MONOTONIC 1
#define CLOCK_MONOTONIC_RAW 4

int clock_gettime(int clock_id, struct timespec* time);
time_t time(time_t* result);
