#pragma once

typedef unsigned long sigset_t;
typedef void (*sighandler_t)(int);

struct sigaction {
    sighandler_t sa_handler;
    sigset_t sa_mask;
    int sa_flags;
};

#define SIGPIPE 13
#define SIG_IGN ((sighandler_t)1)

int sigemptyset(sigset_t* set);
int sigaction(int signal, const struct sigaction* action, struct sigaction* previous);
