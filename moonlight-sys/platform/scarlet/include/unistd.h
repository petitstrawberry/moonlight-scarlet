#pragma once

#include <sys/types.h>

typedef unsigned long useconds_t;

int usleep(useconds_t microseconds);
int close(int descriptor);
