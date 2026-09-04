#pragma once

#include <sys/socket.h>

struct addrinfo {
    int ai_flags;
    int ai_family;
    int ai_socktype;
    int ai_protocol;
    socklen_t ai_addrlen;
    char* ai_canonname;
    struct sockaddr* ai_addr;
    struct addrinfo* ai_next;
};

#define AI_ADDRCONFIG 0x0020
#define EAI_FAIL (-4)
#define EAI_NONAME (-2)

int getaddrinfo(const char* node, const char* service, const struct addrinfo* hints,
                struct addrinfo** result);
void freeaddrinfo(struct addrinfo* result);
