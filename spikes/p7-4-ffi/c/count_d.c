/* Candidate (d): open a fixture, take every entity as a batch, tessellate it into the four arrays and
   print the entity count from the batch and from the entity-id array. */
#include "pvec_d.h"
#include <stdio.h>
#include <stdlib.h>

int main(int argc, char **argv) {
  if (argc < 2) return 2;
  FILE *f = fopen(argv[1], "rb");
  if (!f) return 2;
  fseek(f, 0, SEEK_END);
  long n = ftell(f);
  fseek(f, 0, SEEK_SET);
  uint8_t *buf = malloc((size_t)n);
  if (!buf || fread(buf, 1, (size_t)n, f) != (size_t)n) return 2;
  fclose(f);

  int32_t file = pvec_d_open(buf, (size_t)n);
  if (file < 0) return 3;
  int32_t batch = pvec_d_query_viewport((uint32_t)file, -1e9, -1e9, 1e9, 1e9, UINT32_MAX);
  if (batch < 0) return 4;
  const double chain[15] = {1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 0.25, 1e300};
  int32_t result = pvec_d_tessellate_batch((uint32_t)batch, chain, 15, 4096, 0);
  if (result < 0) return 5;
  int32_t ids = pvec_d_buffer_len((uint32_t)result, 2);
  const uint32_t *id_array = (const uint32_t *)pvec_d_buffer_ptr((uint32_t)result, 2);
  if (ids < 0 || (ids > 0 && !id_array)) return 6;
  printf("%d\n", pvec_d_batch_len((uint32_t)batch));
  if (ids != pvec_d_batch_len((uint32_t)batch)) return 7;
  pvec_d_release((uint32_t)result);
  pvec_d_release((uint32_t)batch);
  pvec_d_release((uint32_t)file);
  free(buf);
  return 0;
}
