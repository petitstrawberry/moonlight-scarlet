#pragma once

#include <stddef.h>
#include <stdint.h>
#include <sys/select.h>
#include <sys/types.h>

typedef uint16_t sa_family_t;

struct sockaddr {
    sa_family_t sa_family;
    char sa_data[14];
};

struct sockaddr_storage {
    sa_family_t ss_family;
    unsigned char __storage[126];
};

#define AF_UNSPEC 0
#define AF_INET 2

#define SOCK_STREAM 1
#define SOCK_DGRAM 2

#define SOL_SOCKET 1
#define SO_REUSEADDR 2
#define SO_ERROR 4
#define SO_BROADCAST 6
#define SO_SNDBUF 7
#define SO_RCVBUF 8
#define SO_RCVTIMEO 20
#define SO_SNDTIMEO 21
#define SO_NONBLOCK 0x1001

#define SHUT_RD 0
#define SHUT_WR 1
#define SHUT_RDWR 2

#define MSG_PEEK 0x02
#define MSG_TRUNC 0x20
#define MSG_NOSIGNAL 0

#define SOMAXCONN 128

int socket(int domain, int type, int protocol);
int bind(int socket, const struct sockaddr* address, socklen_t length);
int connect(int socket, const struct sockaddr* address, socklen_t length);
int listen(int socket, int backlog);
int accept(int socket, struct sockaddr* address, socklen_t* length);
int shutdown(int socket, int how);
int getsockname(int socket, struct sockaddr* address, socklen_t* length);
int getpeername(int socket, struct sockaddr* address, socklen_t* length);
int setsockopt(int socket, int level, int option, const void* value, socklen_t length);
int getsockopt(int socket, int level, int option, void* value, socklen_t* length);
ssize_t send(int socket, const void* buffer, size_t length, int flags);
ssize_t recv(int socket, void* buffer, size_t length, int flags);
ssize_t sendto(int socket, const void* buffer, size_t length, int flags,
               const struct sockaddr* address, socklen_t address_length);
ssize_t recvfrom(int socket, void* buffer, size_t length, int flags,
                 struct sockaddr* address, socklen_t* address_length);
