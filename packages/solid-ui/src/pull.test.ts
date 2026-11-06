import { gapOpen, pullGap, pullProgress, pullRelease, quantizeProgress } from "./pull.ts";

function fail(message: string): never {
  console.error(message);
  process.exit(1);
}

if (pullRelease({ edge: "start", overscroll: -10, threshold: 64, busy: false }) !== "ignore") {
  fail("short pull");
}
if (pullRelease({ edge: "start", overscroll: -64, threshold: 64, busy: false }) !== "refresh") {
  fail("armed pull");
}
if (pullRelease({ edge: "start", overscroll: -80, threshold: 64, busy: true }) !== "hold") {
  fail("busy pull");
}
if (pullRelease({ edge: "start", overscroll: -10, threshold: 64, busy: true }) !== "ignore") {
  fail("busy short pull");
}
if (pullRelease({ edge: "start", overscroll: -0.2, threshold: 64, busy: true }) !== "ignore") {
  fail("closed gap");
}
if (pullRelease({ edge: "end", overscroll: 70, threshold: 64, busy: false }) !== "refresh") {
  fail("bottom pull");
}
if (pullRelease({ edge: "end", overscroll: -70, threshold: 64, busy: false }) !== "ignore") {
  fail("bottom sign");
}
if (pullProgress("start", -32, 64) !== 0.5) {
  fail("progress");
}
if (pullProgress("start", -200, 64) !== 1) {
  fail("clamped progress");
}
if (!gapOpen("end", 1) || gapOpen("end", 0)) {
  fail("gap");
}
if (pullGap("start", 0) !== 0 || pullGap("start", -0.2) !== 0 || pullGap("start", -40) !== 40) {
  fail("pull gap");
}
if (quantizeProgress(0) !== 0 || quantizeProgress(1) !== 1 || quantizeProgress(0.5) !== 0.5) {
  fail("quantize");
}
console.log("pull.test.ts ok");
