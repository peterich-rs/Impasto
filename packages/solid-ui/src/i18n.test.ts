import assert from "node:assert/strict";
import { messagesFor, t } from "./i18n.ts";

assert.deepEqual(messagesFor("zh-CN"), {
  switchOn: "开",
  switchOff: "关",
  retry: "重试",
  back: "返回",
  errorTitle: "出错了",
});
assert.equal(messagesFor("en").switchOn, "On");
assert.equal(messagesFor("fr").switchOff, "Off");
assert.equal(t("retry", undefined, "zh-CN"), "重试");
assert.equal(t("back", { name: "A" }, "en"), "Back");
assert.equal(t("errorTitle", undefined, "en"), "Something went wrong");

console.log("i18n.test.ts ok");
