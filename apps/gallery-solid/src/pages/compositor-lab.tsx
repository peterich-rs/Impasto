import { Show, createSignal, onCleanup } from "solid-js";
import { scene, type NodeHandle } from "@impasto/solid-renderer";
import { createRouter, RouterView } from "@impasto/solid-router";
import { Button, Column, Dialog, HighlightBorder, Image, InsetShadow, OverlayRoot, Ripple, Row, ScrollArea, Sheet, Shimmer, Stack, Text, Vibrancy, View, VirtualList, NavigationBar, bindScrollStyle, createScrollController, interpolate, interpolateColor, useScrollThreshold } from "@impasto/solid-ui";
import { loc } from "../copy.ts";

export const COMPOSITOR_LAB_URL = "impasto://compositor-lab";

const INK = "#15212a";
const SECONDARY = "#46535d";
const PAGE = "#f5f7fa";
const HAIRLINE = "#dce1e7";
const VIRTUAL_ROWS = Array.from({ length: 1_000 }, (_, index) => index + 1);
const SPRING = { stiffness: 14, damping: 7 };

function VirtualScrollLab(props: { onBack?: () => void; auto: boolean; observe?: boolean }): NodeHandle {
  const scroll = createScrollController();
  if (props.observe) onCleanup(scroll.onOffset(() => {}));
  function runCoast() {
    scroll.stop();
    scroll.scrollTo(0, 80);
    scroll.fling(0, -2.8);
    scene.requestFrame();
  }
  const timer = props.auto ? setTimeout(runCoast, 500) : undefined;
  onCleanup(() => { if (timer !== undefined) clearTimeout(timer); });
  return (
    <Column style={{ flex: 1, minHeight: 0, padding: 20, gap: 12, backgroundColor: PAGE }}>
      <Row style={{ minHeight: 40, gap: 8, flexWrap: "wrap", alignItems: "center", justifyContent: "space-between" }}>
        <Text style={{ fontSize: 24, fontWeight: 500, color: INK }}>{loc("虚拟列表", "Virtual list")}</Text>
        <Row style={{ gap: 8 }}>
          {props.onBack && <Button size="compact" label={loc("返回", "Back")} onClick={props.onBack} />}
          <Button size="compact" label={loc("运行滚动", "Run coast")} onClick={runCoast} />
        </Row>
      </Row>
      <VirtualList items={VIRTUAL_ROWS} itemKey={(item) => item} estimatedHeight={52}
        refresh={{ onRefresh: () => {} }} overscan={8} controller={scroll} sticky={(item) => item % 24 === 1}
        style={{ flex: 1, minHeight: 0, borderRadius: 12, backgroundColor: "#ffffff" }}>
        {(item) => (
          <Row style={{ width: "100%", height: 52, flexShrink: 0, paddingLeft: 16, paddingRight: 16,
            alignItems: "center", borderColor: HAIRLINE, borderWidth: 1,
            backgroundColor: item % 24 === 1 ? "#e9edf0" : "#ffffff" }}>
            <Text style={{ color: INK, fontSize: 14 }}>
              {item % 24 === 1 ? loc(`第 ${Math.ceil(item / 24)} 组`, `Section ${Math.ceil(item / 24)}`)
                : loc(`记录 ${item}`, `Record ${item}`)}
            </Text>
          </Row>
        )}
      </VirtualList>
    </Column>
  );
}

function ScrollRulesLab(props: { visuals: boolean; auto?: boolean; onBack?: () => void }): NodeHandle {
  const scroll = createScrollController();
  const pastTop = useScrollThreshold(scroll, { value: 300, enabled: !props.visuals });
  const samples = Array.from({ length: 42 }, (_, index) => index + 1);
  function coast(): void {
    scroll.stop();
    scroll.scrollTo(0, 40);
    scroll.fling(0, -2.1);
    scene.requestFrame();
  }
  if (props.auto) {
    const launch = setTimeout(coast, 750);
    onCleanup(() => clearTimeout(launch));
  }
  return (
    <Column style={{ flex: 1, minHeight: 0, backgroundColor: PAGE }}>
      <Row style={{ height: 52, paddingLeft: 16, paddingRight: 16, gap: 8, alignItems: "center", justifyContent: "space-between" }}>
        {props.onBack ? <Button size="compact" label={loc("返回", "Back")} onClick={props.onBack} /> : null}
        <Text style={{ fontSize: 19, color: INK, fontWeight: 500 }}>
          {props.visuals ? loc("滚动联动", "Scroll-linked visuals") : loc("滚动返顶", "Back to top")}
        </Text>
        <Button size="compact" label={loc("运行滚动", "Run coast")} onClick={coast} />
      </Row>
      <Column style={{ width: "100%", flex: 1, minHeight: 0, overflow: "clip" }}>
        <ScrollArea controller={scroll} style={{ flex: 1, minHeight: 0 }}>
          <Column>
          {props.visuals && (
            <Stack style={{ width: "100%", height: 220, minHeight: 220, flexShrink: 0, overflow: "clip" }}>
              <View
                ref={(node) => {
                  if (node === null) return;
                  onCleanup(bindScrollStyle(node, scroll, {
                    backgroundColor: interpolateColor([0, 180], ["#d1e4dd", "#c1d9e7"]),
                  }));
                }}
                style={{ position: "absolute", top: 0, left: 0, width: "100%", height: "100%" }}
              />
              <Image
                ref={(node) => {
                  if (node === null) return;
                  onCleanup(bindScrollStyle(node, scroll, {
                    translateY: interpolate([0, 200], [0, 96]),
                    scale: interpolate([0, 200], [1, 1.2]),
                    opacity: interpolate([0, 200], [0.46, 0.18]),
                  }));
                }}
                source="thumbhash:XWmGEooooIeSiIigkgjad3B3hoiHeHg="
                style={{ position: "absolute", top: 0, left: 0, width: "100%", height: "100%" }}
              />
              <Text style={{ position: "absolute", left: 20, bottom: 26, fontSize: 28, color: INK }}>
                {loc("滚动视差", "Parallax header")}
              </Text>
            </Stack>
          )}
          {samples.map((index) => (
            <Row style={{ height: 54, flexShrink: 0, alignItems: "center", paddingLeft: 20,
              backgroundColor: index % 2 === 0 ? "#ffffff" : PAGE }}>
              <Text style={{ color: INK, fontSize: 15 }}>{loc(`记录 ${index}`, `Record ${index}`)}</Text>
            </Row>
          ))}
          </Column>
        </ScrollArea>
        {props.visuals && (
          <NavigationBar title={loc("滚动视图", "Scroll view")} scroll={scroll} scrollEffect="fade-backdrop"
            style={{ position: "absolute", top: 0, left: 0, height: 52, paddingLeft: 20, flex: 0 }} />
        )}
        {!props.visuals && (
          <View style={{ position: "absolute", right: 16, bottom: 16, display: pastTop() ? "flex" : "none" }}>
            <Button size="compact" label={loc("回到顶部", "Back to top")}
              onClick={() => scroll.scrollTo(0, 0, { stiffness: 220, damping: 30 })} />
          </View>
        )}
      </Column>
    </Column>
  );
}

function MaterialLab(): NodeHandle {
  return (
    <Column style={{ flex: 1, padding: 20, gap: 16, backgroundColor: PAGE }}>
      <Text style={{ fontSize: 24, fontWeight: 500, color: INK }}>{loc("合成材质", "Compositor materials")}</Text>
      <Shimmer style={{ height: 48, borderRadius: 12, overflow: "hidden" }} />
      <Stack style={{ height: 156, borderRadius: 12, overflow: "hidden", backgroundColor: "#176356" }}>
        <Row style={{ position: "absolute", top: 0, left: 0, width: "100%", height: "100%" }}>
          <View style={{ flex: 2, backgroundColor: "#0d4d45" }} />
          <View style={{ flex: 1, backgroundColor: "#328375" }} />
          <View style={{ flex: 1, backgroundColor: "#4b9b88" }} />
        </Row>
        <Vibrancy progressive={1} style={{ position: "absolute", top: 24, left: 16, right: 16, height: 84,
          borderRadius: 12, overflow: "hidden" }} />
        <Text style={{ position: "absolute", top: 46, left: 32, width: "75%", height: 44,
          fontSize: 28, color: "#ffffff" }}>{loc("色彩与光", "Color and light")}</Text>
        <InsetShadow cornerRadius={12} style={{ position: "absolute", top: 0, left: 0, right: 0, height: "100%" }} />
        <HighlightBorder color={[1, 1, 1, 0.35]} style={{ position: "absolute", top: 0, left: 0, right: 0, height: "100%" }} />
      </Stack>
      <Ripple style={{ width: 168, height: 44, borderRadius: 8, backgroundColor: "#176356",
        justifyContent: "center", alignItems: "center" }}>
        <Text style={{ fontSize: 14, color: "#ffffff" }}>{loc("点按查看涟漪", "Press for ripple")}</Text>
      </Ripple>
    </Column>
  );
}

function TransitionLab(props: { scenario: string }): NodeHandle {
  const [sheet, setSheet] = createSignal(false);
  const [dialog, setDialog] = createSignal(false);
  const picture = "thumbhash:XWmGEooooIeSiIigkgjad3B3hoiHeHg=";
  const summary = (title: string, details?: string) => (
    <Column style={{ width: "100%", minWidth: 0, padding: 20, gap: 12 }}>
      <Text style={{ width: "100%", color: INK, fontSize: 24 }}>{title}</Text>
      <Image source={picture} style={{ width: "100%", height: 112, borderRadius: 12 }} />
      {details && <Text style={{ width: "100%", color: SECONDARY, fontSize: 15 }}>{details}</Text>}
    </Column>
  );
  const router = createRouter([
    {
      path: "/",
      component: () => (
        <Column style={{ flex: 1, backgroundColor: PAGE }}>
          {summary(loc("转场", "Transitions"), loc("图文页面与弹层", "Pages and overlays with text and images"))}
          <Column style={{ padding: 20, gap: 8 }}>
            <Button label={loc("打开下一页", "Open next page")} onClick={() => router.push("/next")} />
            <Button label={loc("打开底部面板", "Open bottom sheet")} onClick={() => setSheet(true)} />
            <Button label={loc("打开对话框", "Open dialog")} onClick={() => setDialog(true)} />
          </Column>
        </Column>
      ),
    },
    {
      path: "/next",
      component: () => (
        <Column style={{ flex: 1, backgroundColor: "#ffffff" }}>
          {summary(loc("下一页", "Next page"), loc("验证页面 Push 与 Pop", "Push and pop this page"))}
          <Button label={loc("返回", "Back")} onClick={() => router.pop()} />
        </Column>
      ),
    },
  ], { linking: false });
  const actions = [
    { name: "router", enter: () => router.push("/next"), leave: () => router.pop() },
    { name: "sheet", enter: () => setSheet(true), leave: () => setSheet(false) },
    { name: "dialog", enter: () => setDialog(true), leave: () => setDialog(false) },
  ];
  const timers = actions.filter(({ name }) => !props.scenario || name === props.scenario)
    .flatMap(({ enter, leave }, index) => [
      setTimeout(enter, 600 + index * 1800),
      setTimeout(leave, (props.scenario ? 2200 : 1500) + index * 1800),
    ]);
  onCleanup(() => { for (const timer of timers) clearTimeout(timer); });
  return (
    <OverlayRoot>
      <RouterView router={router} background={PAGE} />
      <Sheet open={sheet()} onClose={() => setSheet(false)} style={{ width: "100%" }}>
        {summary(loc("底部面板", "Bottom sheet"))}
      </Sheet>
      <Dialog open={dialog()} onClose={() => setDialog(false)}
        style={{ width: 300, maxWidth: "90%", borderRadius: 12, backgroundColor: "#ffffff" }}>
        {summary(loc("对话框", "Dialog"))}
      </Dialog>
    </OverlayRoot>
  );
}

/** Runs deterministic Gallery motion samples through the normal components. */
export function CompositorLabPage(props: { onBack?: () => void; launchUrl: string | null }): NodeHandle {
  if (props.launchUrl === `${COMPOSITOR_LAB_URL}/materials`) {
    return <MaterialLab />;
  }
  if (props.launchUrl === `${COMPOSITOR_LAB_URL}/scroll-threshold`) {
    return <ScrollRulesLab visuals={false} auto onBack={props.onBack} />;
  }
  if (props.launchUrl === `${COMPOSITOR_LAB_URL}/scroll-style`) {
    return <ScrollRulesLab visuals auto onBack={props.onBack} />;
  }
  if (props.launchUrl === `${COMPOSITOR_LAB_URL}/virtual-list`
    || props.launchUrl === `${COMPOSITOR_LAB_URL}/virtual-list-observed`) {
    return <VirtualScrollLab auto onBack={props.onBack} observe={props.launchUrl.endsWith("-observed")} />;
  }
  const transitionsUrl = `${COMPOSITOR_LAB_URL}/transitions`;
  if (props.launchUrl === transitionsUrl || props.launchUrl?.startsWith(`${transitionsUrl}/`)) {
    return <TransitionLab scenario={props.launchUrl.slice(transitionsUrl.length + 1)} />;
  }
  const [rulesLab, setRulesLab] = createSignal<"threshold" | "style" | null>(null);
  const [status, setStatus] = createSignal(loc("准备就绪", "Ready"));
  let card: NodeHandle | null = null;
  let scroll: NodeHandle | null = null;
  let targetOnRight = false;
  let transformEnd = 0;
  let coastEnd = 0;
  const sections = [
    loc("顶部", "Top"), loc("上段", "Upper section"),
    loc("中段", "Middle section"), loc("下段", "Lower section"),
  ];

  function runTransform() {
    if (card === null) return;
    const node = card;
    const distance = Math.max(32, Math.min(72, scene.rect(node).w * 0.65));
    targetOnRight = !targetOnRight;
    const target = targetOnRight ? 1 : 0;
    const tracks = [
      ["translateX", -52 - distance, -52 + distance],
      ["translateY", -36, -20],
      ["rotate", -0.18, 0.22],
      ["scale", 1, 0.86],
      ["opacity", 242, 168],
    ] as const;
    setStatus(loc("变换进行中", "Transform running"));
    for (const [key, left, right] of tracks) {
      scene.animate(node, key, target ? left : right, target ? right : left,
        SPRING.stiffness, SPRING.damping, true);
    }
    scene.requestFrame();
  }

  function runCoast() {
    if (scroll === null) return;
    scene.stopScroll(scroll);
    scene.setScrollOffset(scroll, 0, 80);
    scene.fling(scroll, 0, -1.7);
    scene.requestFrame();
    setStatus(loc("滚动惯性进行中", "Scroll coast running"));
  }

  function runSlowFade() {
    if (card === null) return;
    scene.animate(card, "opacity", 242, 140, 4, 4);
    scene.requestFrame();
  }

  const auto = props.launchUrl === `${COMPOSITOR_LAB_URL}/transform`
    ? runTransform
    : props.launchUrl === `${COMPOSITOR_LAB_URL}/scroll` ? runCoast
    : props.launchUrl === `${COMPOSITOR_LAB_URL}/slow-fade` ? runSlowFade
    : null;
  const timer = auto === null ? undefined : setTimeout(auto, 500);
  onCleanup(() => {
    if (timer !== undefined) clearTimeout(timer);
    if (transformEnd !== 0) scene.offMotionEnd(transformEnd);
    if (coastEnd !== 0) scene.offMotionEnd(coastEnd);
  });

  return (
    <Show when={rulesLab()} keyed fallback={
    <Column style={{ flex: 1, minHeight: 0, backgroundColor: PAGE }}>
      <Row style={{ height: 56, paddingLeft: 20, paddingRight: 12, alignItems: "center",
        justifyContent: "space-between", borderColor: HAIRLINE, borderWidth: 1 }}>
        <Text style={{ fontSize: 24, fontWeight: 500, color: INK }}>
          {loc("合成动画", "Compositor animation")}
        </Text>
        {props.onBack && <Button size="compact" label={loc("返回", "Back")} onClick={props.onBack} />}
      </Row>
      <Column style={{ width: "100%", maxWidth: 640, alignSelf: "center", flex: 1,
        minHeight: 0, padding: 20, gap: 12 }}>
        <Row style={{ gap: 8, flexWrap: "wrap" }}>
          <Button size="compact" label={loc("滚动返顶", "Back to top")} onClick={() => setRulesLab("threshold")} />
          <Button size="compact" label={loc("滚动联动", "Scroll-linked visuals")} onClick={() => setRulesLab("style")} />
        </Row>
        <Stack style={{ height: 148, backgroundColor: "#e9edf0", borderRadius: 12, overflow: "hidden",
          alignItems: "center", justifyContent: "center" }}>
          <View style={{ width: "76%", height: 1, backgroundColor: "#a7b7b8" }} />
          <View ref={(node) => {
            if (node === null) return;
            card = node;
            transformEnd = scene.onMotionEnd(node, "translateX", () => {
              setStatus(loc("变换已结束", "Transform settled"));
            });
          }}
          onClick={runTransform}
          accessibilityLabel={loc("反向播放变换", "Reverse transform")}
          style={{ position: "absolute", left: "50%", top: "50%", width: 104, height: 56,
            translateX: -52, translateY: -28, opacity: 0.95, rotate: -0.18,
            transformOriginX: "50%", transformOriginY: "50%",
            borderRadius: 8, backgroundColor: "#176356", alignItems: "center", justifyContent: "center" }}>
            <Text style={{ color: "#ffffff", fontSize: 13 }}>{loc("点按反向", "Tap to reverse")}</Text>
          </View>
        </Stack>
        <Row style={{ gap: 8, flexWrap: "wrap" }}>
          <Button size="compact" label={loc("运行变换", "Run transform")} onClick={runTransform} />
          <Button size="compact" label={loc("运行滚动", "Run coast")} onClick={runCoast} />
        </Row>
        <Text style={{ color: SECONDARY, fontSize: 13 }}>{status()}</Text>
        <ScrollArea viewportRef={(node) => {
          if (node === null) return;
          scroll = node;
          coastEnd = scene.onMotionEnd(node, "scrollY", () => {
            setStatus(loc("滚动惯性已结束", "Scroll coast settled"));
          });
        }} style={{ flex: 1, minHeight: 0, borderRadius: 12, backgroundColor: "#ffffff" }}>
          <Column style={{ gap: 0 }}>
            {sections.map((section) => (
              <Column style={{ height: 400, flexShrink: 0, padding: 12, gap: 76,
                borderWidth: 1, borderColor: HAIRLINE }}>
                <Text style={{ color: INK, fontSize: 14 }}>{section}</Text>
                <View style={{ width: "100%", height: 1, backgroundColor: HAIRLINE }} />
                <View style={{ width: "100%", height: 1, backgroundColor: HAIRLINE }} />
                <View style={{ width: "100%", height: 1, backgroundColor: HAIRLINE }} />
              </Column>
            ))}
          </Column>
        </ScrollArea>
      </Column>
    </Column>
    }>
      {(lab) => <ScrollRulesLab visuals={lab === "style"} onBack={() => setRulesLab(null)} />}
    </Show>
  );
}
