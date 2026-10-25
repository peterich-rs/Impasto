import { host, onFrame, requestFrame } from "@uifw/solid-renderer";
import { bindMotion, installMotionTestHooks, transition } from "./motion-clock.ts";

bindMotion({
  reduceMotion: () => host.reduceMotion(),
  onFrame,
  requestFrame,
});

export { installMotionTestHooks, transition };
