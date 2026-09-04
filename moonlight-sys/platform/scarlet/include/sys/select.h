#pragma once

#include <sys/time.h>

typedef struct { unsigned long bits[16]; } fd_set;

#define FD_ZERO(set) ((void)(set))
#define FD_SET(fd, set) ((void)(fd), (void)(set))
#define FD_CLR(fd, set) ((void)(fd), (void)(set))
#define FD_ISSET(fd, set) (0)

int select(int count, fd_set* read_set, fd_set* write_set, fd_set* error_set,
           struct timeval* timeout);
