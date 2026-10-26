/* Candidate (a): the header written by hand. Nothing checks it against the Rust source. */
#ifndef PVEC_A_H
#define PVEC_A_H

#include <stddef.h>
#include <stdint.h>

typedef struct PvecHandle PvecHandle;

typedef enum PvecStatus {
  PVEC_OK = 0,
  PVEC_BAD_ARGUMENT = -1,
  PVEC_READ = -2,
} PvecStatus;

PvecStatus pvec_open(const uint8_t *bytes, size_t len, PvecHandle **out);
PvecStatus pvec_entity_count(const PvecHandle *handle, uint64_t *out);
void pvec_close(PvecHandle *handle);

#endif
