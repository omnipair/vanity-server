#ifndef VANITY_H
#define VANITY_H

#include <stdint.h>
#include "utils.h"

extern "C" void vanity_round(int gpus, uint8_t *seed, uint8_t *base, uint8_t *owner, char *target, char *suffix, uint64_t target_len, uint64_t suffix_len, uint8_t *out, bool case_insensitive);
__global__ void vanity_search(uint8_t *buffer, uint64_t stride);
__device__ bool matches_target(unsigned char *a, unsigned char *target, uint64_t n, unsigned char *suffix, uint64_t suffix_len, ulong encoded_len);
#endif