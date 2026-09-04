#pragma once

#define FIONBIO 0x5421

int ioctl(int descriptor, unsigned long request, ...);
