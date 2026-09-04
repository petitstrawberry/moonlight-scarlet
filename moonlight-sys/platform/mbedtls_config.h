#pragma once

/* Start from the upstream Mbed TLS 3.6 LTS configuration. */
#include <mbedtls/mbedtls_config.h>

#if defined(LC_SCARLET)
/* Scarlet supplies entropy through its native GetRandom syscall. */
#define MBEDTLS_NO_PLATFORM_ENTROPY
#define MBEDTLS_ENTROPY_HARDWARE_ALT

/* No filesystem-backed persistent PSA key store is used by Moonlight. */
#undef MBEDTLS_FS_IO
#undef MBEDTLS_PSA_CRYPTO_STORAGE_C
#undef MBEDTLS_PSA_ITS_FILE_C

/* These acceleration paths are specific to x86 CPUs. */
#undef MBEDTLS_AESNI_C
#undef MBEDTLS_AESCE_C
#undef MBEDTLS_PADLOCK_C
#undef MBEDTLS_TIMING_C

/* Streaming crypto does not require calendar time. */
#undef MBEDTLS_HAVE_TIME
#undef MBEDTLS_HAVE_TIME_DATE
#endif
