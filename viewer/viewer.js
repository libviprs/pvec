// Placeholder until the viewer is written (issue #271).
window.pvecViewer = {
  settled: true,
  view: { cx: 0, cy: 0, scale: 1 },
  idle: async () => {},
  toScreen: () => [NaN, NaN],
  sourceAt: () => [NaN, NaN],
};
