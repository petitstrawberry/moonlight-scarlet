#pragma once

#define HUGE_VAL (__builtin_huge_val())
#define HUGE_VALF (__builtin_huge_valf())
#define INFINITY (__builtin_inff())
#define NAN (__builtin_nanf(""))

double atan(double value);
double atan2(double y, double x);
double ceil(double value);
double cos(double value);
double exp(double value);
double fabs(double value);
double floor(double value);
double log(double value);
double log10(double value);
double pow(double base, double exponent);
double sin(double value);
double sqrt(double value);

float atan2f(float y, float x);
float ceilf(float value);
float cosf(float value);
float expf(float value);
float fabsf(float value);
float floorf(float value);
float logf(float value);
float log10f(float value);
float powf(float base, float exponent);
float sinf(float value);
float sqrtf(float value);

long lrint(double value);
long lrintf(float value);
