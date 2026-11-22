// The Node fixture passes official Solid 2.0.0 and the QuickJS test passes the
// native graph's adapter. Both compare the printed lines with order.txt.
export function runProbe(log, solid) {
  const {
    createMemo,
    createRenderEffect,
    createRoot,
    createSignal,
    createTrackedEffect,
    flush,
    onCleanup,
    untrack,
  } = solid;

  createRoot(() => {
    const [source, setSource] = createSignal(1);
    createTrackedEffect(() => log(`tracked ${source()}`));
    createRenderEffect(
      () => source(),
      (value) => log(`render ${value}`),
    );
    log("--- created ---");
    setSource(2);
    flush();
    log("--- flushed ---");
  });

  createRoot(() => {
    const [source, setSource] = createSignal(1);
    let runs = 0;
    const derived = createMemo(() => {
      runs += 1;
      return source() + 1;
    });
    derived();
    derived();
    log(`memo reads ${runs}`);
    setSource(4);
    flush();
    derived();
    log(`memo after write ${runs}`);
  });

  createRoot(() => {
    const [tracked, setTracked] = createSignal(1);
    const [ignored, setIgnored] = createSignal(10);
    const derived = createMemo(() => tracked() + untrack(ignored));
    log(`untrack ${derived()}`);
    setIgnored(100);
    flush();
    log(`untrack ignored ${derived()}`);
    setTracked(2);
    flush();
    log(`untrack tracked ${derived()}`);
  });

  createRoot((dispose) => {
    onCleanup(() => log("first"));
    onCleanup(() => log("second"));
    dispose();
    log("--- disposed ---");
  });

  createRoot(() => {
    const [source, setSource] = createSignal(1);
    createRenderEffect(
      () => source(),
      (value) => {
        log(`run ${value}`);
        return () => log(`cleanup ${value}`);
      },
    );
    log("--- effect created ---");
    setSource(2);
    flush();
  });

  createRoot(() => {
    const [source, setSource] = createSignal(0);
    createRenderEffect(
      () => source(),
      (value) => log(`batch ${value}`),
    );
    log("--- batch created ---");
    flush(() => {
      setSource(1);
      setSource(2);
    });
    log("--- batch flushed ---");
  });
}
