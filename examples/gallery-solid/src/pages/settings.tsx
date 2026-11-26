import { createSignal, Show } from "solid-js";
import { Column, Dialog, Row, ScrollArea, View } from "@impasto/solid-ui";
import type { NodeHandle } from "@impasto/solid-renderer";
import { store as devtools } from "@impasto/devtools";
import { loc } from "../copy.ts";
import { settingsSig, updateSettings, type Updown } from "../state/settings.ts";
import { accountRef } from "../state/account-ref.ts";
import { showToast } from "../state/toast.ts";
import { Action, colors, filling, Glyph, Label, Rule, Section, Segments, tone } from "../widgets/stocks/ui.tsx";

const updowns: readonly Updown[] = ["cn", "intl"];
const locales: readonly ("zh" | "en")[] = ["zh", "en"];
const speeds: readonly number[] = [1, 60, 240];

export function SettingsPage(_props: { router: unknown }): NodeHandle {
  const account = accountRef();
  const [confirmReset, setConfirmReset] = createSignal(false);
  const updownLabel = (value: Updown) => value === "cn" ? loc("红涨绿跌", "Red up") : loc("绿涨红跌", "Green up");
  const speedLabel = (scale: number) => scale === 1 ? loc("实时", "Realtime") : `${scale}×`;
  return <Column style={[filling, { backgroundColor: colors.bg }]}>
    <ScrollArea style={filling}>
    <Section title={loc("设置", "Settings")}>
      <Show when={devtools.available()}><Glyph name="more" title={loc("调试", "Devtools")} onClick={() => devtools.open()} size={18} color={colors.dim}
        compact style={{ alignSelf: "flex-end", backgroundColor: "#00000000" }} /></Show>
    </Section>
    <Rule />
    <Section title={loc("涨跌配色", "Up/down colors")} note={loc("示例 上涨 · 下跌", "sample up · down")}>
      <Segments options={updowns} selected={settingsSig().updown} label={updownLabel} onSelect={(value) => updateSettings({ updown: value })} />
      <Row style={{ gap: 6, alignItems: "baseline" }}>
        <Label size={13} color={tone(1.5)}>{loc("+1.50%", "+1.50%")}</Label>
        <Label size={13} color={tone(-1.5)}>{loc("-1.50%", "-1.50%")}</Label>
      </Row>
    </Section>
    <Rule />
    <Section title={loc("语言", "Language")}>
      <Segments options={locales} selected={settingsSig().locale} label={(value) => value === "zh" ? "中文" : "English"} onSelect={(value) => updateSettings({ locale: value })} />
    </Section>
    <Rule />
    <Section title={loc("行情速度", "Market speed")} note={loc("模拟时钟倍率", "simulation clock")}>
      <Segments options={speeds} selected={settingsSig().timeScale} label={speedLabel}
        onSelect={(value) => updateSettings({ timeScale: value })} />
    </Section>
    <Rule />
    <Section title={loc("模拟账户", "Paper account")}>
      <Column style={{ gap: 8 }}>
        <Action label={loc("重置模拟账户", "Reset paper account")} onClick={() => setConfirmReset(true)} />
        <Label size={11} color={colors.faint} style={{ maxLines: 2 }}>{loc("只清空资金、持仓和委托，自选与设置保留", "Clears cash, positions, and orders only")}</Label>
      </Column>
    </Section>
    <Rule />
    <Section>
      <Label size={12} color={colors.faint} style={{ textAlign: "center" }}>{loc("Impasto Quotes · 模拟盘 · 数据均为虚构", "Impasto Quotes · paper trading · fictional data")}</Label>
    </Section>
    {/* Clears the floating route capsule. */}
    <View style={{ height: 72, flexShrink: 0 }} />
    </ScrollArea>
    <Dialog open={confirmReset()} onClose={() => setConfirmReset(false)} title={loc("重置账户", "Reset account")}>
      <Column style={{ gap: 12 }}>
        <Label size={14} style={{ maxLines: 3 }}>{loc("确认清空模拟账户？自选和设置不会改动。", "Reset the paper account? Watchlist and settings stay.")}</Label>
        <Row style={{ gap: 8 }}>
          <Action label={loc("取消", "Cancel")} onClick={() => setConfirmReset(false)} style={{ flex: 1 }} />
          <Action label={loc("确认重置", "Reset")} primary onClick={() => { account.reset(); setConfirmReset(false); showToast(loc("账户已重置", "Account reset")); }} style={{ flex: 1 }} />
        </Row>
      </Column>
    </Dialog>
  </Column>;
}
