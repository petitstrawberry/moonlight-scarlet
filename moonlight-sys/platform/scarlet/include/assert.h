#pragma once

#include <ScarletBridge.h>

#ifdef NDEBUG
#define assert(condition) ((void)0)
#else
#define assert(condition) ((condition) ? (void)0 : scarlet_abort())
#endif
