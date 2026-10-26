/* Candidate (d): the packed-buffer API for C, written by hand. Handles are integers, arrays are numbers. */
#ifndef PVEC_D_H
#define PVEC_D_H

#include <stddef.h>
#include <stdint.h>

uint32_t pvec_d_abi_version(void);
int32_t pvec_d_open(const uint8_t *bytes, size_t len);
int32_t pvec_d_query_viewport(uint32_t file, double min_x, double min_y, double max_x, double max_y, uint32_t budget);
int32_t pvec_d_batch_len(uint32_t batch);
int32_t pvec_d_tessellate_batch(uint32_t batch, const double *chain, size_t chain_len, uint32_t max_segments, uint32_t precision);
int32_t pvec_d_buffer_len(uint32_t result, uint32_t which);
const void *pvec_d_buffer_ptr(uint32_t result, uint32_t which);
int32_t pvec_d_release(uint32_t handle);

#endif
