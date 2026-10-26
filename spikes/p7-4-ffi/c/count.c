/* Candidates (a) and (b): open a fixture through the extern "C" facade and print its entity count.
   HEADER names the header under test: pvec_a.h (hand-written) or pvec_b.h (cbindgen). */
#include HEADER
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

  PvecHandle *h = NULL;
  if (pvec_open(buf, (size_t)n, &h) != 0) return 3;
  uint64_t count = UINT64_MAX; /* a slot the callee has to overwrite whole */
  if (pvec_entity_count(h, &count) != 0) return 4;
  printf("%llu\n", (unsigned long long)count);
  pvec_close(h);
  free(buf);
  return 0;
}
