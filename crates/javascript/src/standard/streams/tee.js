// Default streams share each chunk. Each branch owns its own bounded queue.
(() => {
  const Stream = ReadableStream;
  const getReader = Stream.prototype.getReader;
  Object.defineProperty(Stream.prototype, "tee", {
    configurable: true,
    writable: true,
    value: function tee() {
      const reader = getReader.call(this);
      const controllers = [];
      const cancelled = [false, false];
      const reasons = [];
      let reading;
      let finishCancel;
      let failCancel;
      let finished = false;
      const cancelResult = new Promise((resolve, reject) => {
        finishCancel = resolve;
        failCancel = reject;
      });

      function finish(error, failed) {
        if (finished) return;
        finished = true;
        for (let i = 0; i < 2; ++i) {
          if (cancelled[i]) continue;
          if (failed) controllers[i].error(error);
          else controllers[i].close();
        }
        reader.releaseLock();
        finishCancel();
      }

      function pull() {
        if (finished) return;
        if (!reading) {
          reading = reader.read().then(({ done, value }) => {
            if (finished) return;
            if (done) {
              finish(undefined, false);
              return;
            }
            for (let i = 0; i < 2; ++i) {
              if (!cancelled[i]) controllers[i].enqueue(value);
            }
          }).catch(error => finish(error, true)).finally(() => { reading = undefined; });
        }
        return reading;
      }

      return [0, 1].map(i => new Stream({
        start(controller) { controllers[i] = controller; },
        pull,
        cancel(reason) {
          cancelled[i] = true;
          reasons[i] = reason;
          if (cancelled[0] && cancelled[1] && !finished) {
            finished = true;
            reader.cancel(reasons).then(finishCancel, failCancel).finally(() => reader.releaseLock());
          }
          return cancelResult;
        },
      }, { highWaterMark: 0 }));
    },
  });
})();
