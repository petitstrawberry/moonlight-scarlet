#pragma once

#define F_GETFL 3
#define F_SETFL 4

int fcntl(int descriptor, int command, ...);
