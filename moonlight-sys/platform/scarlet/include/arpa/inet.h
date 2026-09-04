#pragma once

#include <netinet/in.h>
#include <sys/socket.h>

#define INET_ADDRSTRLEN 16

int inet_pton(int family, const char* source, void* destination);
const char* inet_ntop(int family, const void* source, char* destination, socklen_t length);
