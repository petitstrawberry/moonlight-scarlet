#pragma once

#include <sys/types.h>

struct timeval {
    time_t tv_sec;
    long tv_usec;
};

int gettimeofday(struct timeval* time, void* timezone);
