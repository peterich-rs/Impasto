import { createSignal, For, Show } from "solid-js";
import { Column, Row, Field, Pressable, Role, type FieldController } from "@impasto/solid-ui";
import type { NodeHandle } from "@impasto/solid-renderer";
import type { Router } from "@impasto/solid-router";
import { loc } from "../copy.ts";
import { fmtPrice } from "../format.ts";
import type { Instrument } from "../engine/types.ts";
import { useBinder } from "../state/quotes-provider.tsx";
import { inWatchlist, toggleWatch } from "../state/watchlist.ts";
import { Action, change, code, colors, filling, Glyph, Label, PriceTag, Rule, Section } from "../widgets/stocks/ui.tsx";

const HOT = ["00700.HK", "AAPL.US", "600519.CN", "NVDA.US", "TSLA.US", "09988.HK"];
const DEBOUNCE_MS = 200;

export function SearchPage(props: { router: Router }): NodeHandle {
  const { engine } = useBinder();
  const [term, setTerm] = createSignal("");
  const [query, setQuery] = createSignal("");
  let debounce: ReturnType<typeof setTimeout> | undefined;
  let field: FieldController | null = null;
  const results = () => {
    const text = query().trim().toLowerCase();
    if (text.length === 0) return [] as Instrument[];
    return engine.instruments().filter((instrument) => instrument.symbol.toLowerCase().includes(text) || instrument.name.toLowerCase().includes(text)).slice(0, 16);
  };
  const open = (symbol: string) => {
    clearTimeout(debounce);
    field?.load(symbol);
    props.router.push(`/stock/${symbol}`);
  };
  return <Column style={[filling, { backgroundColor: colors.bg }]}>
    <Row style={{ minHeight: 52, paddingLeft: 8, paddingRight: 12, alignItems: "center", gap: 8 }}>
      <Glyph name="back" title={loc("返回", "Back")} onClick={() => props.router.pop()} />
      <Label size={18} weight={600} style={{ flex: 1 }}>{loc("搜索", "Search")}</Label>
    </Row>
    <Rule />
    <Column style={{ padding: 16, gap: 12, flex: 1, minHeight: 0 }}>
      <Field placeholder={loc("代码或名称", "Symbol or name")} action="search" label={loc("搜索", "Search")}
        controller={(ctl) => { field = ctl; }}
        onChange={(value) => { clearTimeout(debounce); debounce = setTimeout(() => setQuery(value), DEBOUNCE_MS); }}
        onSubmit={(value) => { clearTimeout(debounce); setQuery(value); }} />
      <Show when={query().trim().length === 0} fallback={<For each={results()}>{(instrument) => <HitRow instrument={instrument} engine={engine} onOpen={open} />}</For>}>
        <Section title={loc("热门", "Hot")}>
          <Row style={{ gap: 8, flexWrap: "wrap" }}>
            <For each={HOT}>{(symbol) => <Action label={code(symbol)} onClick={() => open(symbol)} />}</For>
          </Row>
        </Section>
      </Show>
      <Show when={query().trim().length > 0 && results().length === 0}>
        <Label color={colors.dim} style={{ padding: 16, textAlign: "center" }}>{loc("没有匹配的标的", "No matching symbols")}</Label>
      </Show>
    </Column>
  </Column>;
}

function HitRow(props: { instrument: Instrument; engine: ReturnType<typeof useBinder>["engine"]; onOpen: (symbol: string) => void }) {
  const saved = () => inWatchlist(props.instrument.symbol);
  return <Column style={{ gap: 0 }}>
    <Pressable role={Role.Button} onClick={() => props.onOpen(props.instrument.symbol)} style={{ paddingLeft: 0, paddingRight: 0, paddingTop: 10, paddingBottom: 10, flexDirection: "row", gap: 10, alignItems: "center" }}>
      <Column style={{ flex: 1, minWidth: 0, gap: 3 }}>
        <Row style={{ gap: 6, alignItems: "baseline" }}>
          <Label size={15} weight={600}>{code(props.instrument.symbol)}</Label>
          <Label size={12} color={colors.dim}>{props.instrument.name}</Label>
        </Row>
        <Label size={11} color={colors.faint}>{props.instrument.symbol}</Label>
      </Column>
      <Glyph name={saved() ? "starFilled" : "star"} title={saved() ? loc("移出自选", "Remove from watchlist") : loc("加入自选", "Add to watchlist")}
        onClick={() => toggleWatch(props.instrument.symbol)} size={18} color={saved() ? colors.amber : colors.faint} />
      <Price value={props.instrument.symbol} engine={props.engine} />
    </Pressable>
    <Rule />
  </Column>;
}

function Price(props: { value: string; engine: ReturnType<typeof useBinder>["engine"] }) {
  return <PriceTag value={fmtPrice(props.engine.quote(props.value).last)} change={change(props.engine.quote(props.value))} />;
}
