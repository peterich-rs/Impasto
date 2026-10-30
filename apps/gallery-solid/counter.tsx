import { Show, createSignal } from "solid-js";
import { Text, boot, directionFor, host } from "@impasto/solid-renderer";
import { RouterView, createRouter, type RouteParams, type Router } from "@impasto/solid-router";
import {
  Button,
  Dialog,
  ErrorPage,
  Field,
  OverlayRoot,
  RefreshControl,
  ScrollArea,
  VirtualList,
  createScrollController,
  messageOf,
  stackOf,
  useTheme,
} from "@impasto/solid-ui";
import { kicker } from "./util.js";

const GUTTER = 20;

function Screen(props: { children?: unknown }) {
  const theme = useTheme();
  return (
    <decorated color={theme().color.background} fill>
      <padding top={12} right={GUTTER} bottom={16} left={GUTTER} fill>
        <column gap={theme().space.l} fill w="100%">
          {props.children}
        </column>
      </padding>
    </decorated>
  );
}

function Title(props: { title: string }) {
  const theme = useTheme();
  const direction = () => directionFor(theme().locale);
  return (
    <column gap={theme().space.s}>
      <Text
        fontSize={theme().typography.caption.size}
        color={theme().color.textMuted}
        weight={theme().typography.caption.weight}
        direction={direction()}
      >
        {kicker()}
      </Text>
      <Text
        fontSize={theme().typography.headline.size}
        color={theme().color.text}
        weight={theme().typography.headline.weight}
        direction={direction()}
      >
        {props.title}
      </Text>
    </column>
  );
}

function TabBar(props: { router: Router }) {
  const theme = useTheme();
  const selected = () => {
    const list = props.router.stack();
    const top = list[list.length - 1];
    const path = top?.path ?? "";
    if (path.startsWith("/list")) {
      return "list";
    }
    if (path.startsWith("/nav")) {
      return "nav";
    }
    return "schedule";
  };
  return (
    <decorated color={theme().color.surface} radius={theme().shape.full} w="100%">
      <padding edges={theme().space.s} w="100%">
        <row gap={theme().space.m} justify="space-evenly" w="100%">
          <Button
            label="调度"
            variant={selected() === "schedule" ? "primary" : "quiet"}
            onClick={() => props.router.reset("/schedule")}
          />
          <Button
            label="导航"
            variant={selected() === "nav" ? "primary" : "quiet"}
            onClick={() => props.router.reset("/nav/1")}
          />
          <Button
            label="列表"
            variant={selected() === "list" ? "primary" : "quiet"}
            onClick={() => props.router.reset("/list")}
          />
        </row>
      </padding>
    </decorated>
  );
}

function SchedulePage(_props: { params: RouteParams }) {
  const theme = useTheme();
  const [label, setLabel] = createSignal("还是旧的");
  return (
    <column gap={theme().space.l} w="100%">
      <Title title="调度" />
      <Text
        fontSize={theme().typography.display.size}
        color={theme().color.text}
        weight={theme().typography.display.weight}
        direction={directionFor(theme().locale)}
      >
        {label()}
      </Text>
      <Text
        fontSize={theme().typography.body.size}
        color={theme().color.textMuted}
        direction={directionFor(theme().locale)}
      >
        微任务在下一帧把这段文字写回来。
      </Text>
      <Field placeholder="标签" value={label()} onChange={setLabel} />
      <Button
        wide
        variant="primary"
        label="微任务改字"
        onClick={() => {
          queueMicrotask(() => setLabel("已经改了"));
        }}
      />
    </column>
  );
}

const LIST_ITEMS = Array.from({ length: 200 }, (_, n) => ({ n }));

function ListPage(_props: { params: RouteParams }) {
  const theme = useTheme();
  const list = createScrollController();
  const [scale, setScale] = createSignal(1);
  const [missing, setMissing] = createSignal(false);
  const [name, setName] = createSignal("");
  const [refreshed, setRefreshed] = createSignal(0);
  return (
    <column gap={theme().space.l} fill w="100%">
      <Title title="列表" />
      <row gap={theme().space.s} w="100%">
        <Button
          wide
          label={scale() === 1 ? "字号 2.0" : "字号 1.0"}
          onClick={() => {
            const next = scale() === 1 ? 2 : 1;
            setScale(next);
            host.setTextScale(next);
          }}
        />
        <Button wide label={missing() ? "去掉缺图" : "缺图"} onClick={() => setMissing(!missing())} />
      </row>
      <Show when={missing()}>
        <image source="asset:missing" w={32} h={32} />
      </Show>
      <Field placeholder="姓名" onChange={setName} />
      <Text
        fontSize={theme().typography.body.size}
        color={theme().color.text}
        direction={directionFor(theme().locale)}
      >
        {name() || "未填写"}
      </Text>
      <clip radius={theme().shape.l} fill>
        <decorated color={theme().color.surface} fill>
          <RefreshControl controller={list} onRefresh={() => setRefreshed((count) => count + 1)}>
            <Text
              fontSize={theme().typography.caption.size}
              color={theme().color.textMuted}
              direction={directionFor(theme().locale)}
            >
              {refreshed() === 0 ? "下拉刷新" : `已刷新 ${refreshed()}`}
            </Text>
            <Text
              fontSize={theme().typography.caption.size}
              color={theme().color.textMuted}
              direction={directionFor(theme().locale)}
            >
              横向分页
            </Text>
            <ScrollArea
              axis="horizontal"
              physics={{ snapX: { interval: 72, align: "start" }, overscroll: "clamp" }}
            >
              <row gap={8}>
                <sized w={72} h={36} />
                <sized w={72} h={36} />
                <sized w={72} h={36} />
                <sized w={72} h={36} />
                <sized w={72} h={36} />
              </row>
            </ScrollArea>
          </RefreshControl>
          <VirtualList
            items={LIST_ITEMS}
            itemKey={(item) => item.n}
            estimatedHeight={48}
            controller={list}
            physics={{ overscroll: "rubber" }}
            recycle={{
              typeOf: () => "row",
              render: (item) => (
                <padding
                  left={theme().space.l}
                  right={theme().space.l}
                  top={theme().space.m}
                  bottom={theme().space.m}
                >
                  <Text
                    fontSize={theme().typography.body.size}
                    color={theme().color.text}
                    direction={directionFor(theme().locale)}
                  >
                    {`Item ${item().n}`}
                  </Text>
                </padding>
              ),
            }}
          />
        </decorated>
      </clip>
    </column>
  );
}

function App() {
  const theme = useTheme();
  const router = createRouter(
    [
      { path: "/schedule", component: SchedulePage },
      {
        path: "/nav/:depth",
        component: (props) => {
          const depth = Number(props.params.depth ?? "1") || 1;
          const [dialog, setDialog] = createSignal(false);
          return (
            <column gap={theme().space.l} w="100%">
              <Title title={`第 ${depth} 页`} />
              <Text
                fontSize={theme().typography.body.size}
                color={theme().color.textMuted}
                direction={directionFor(theme().locale)}
              >
                推进会压入下一层，弹出等转场结束再卸下。
              </Text>
              <Button wide variant="primary" label="推进" onClick={() => router.push(`/nav/${depth + 1}`)} />
              <row gap={theme().space.s} w="100%">
                <Button wide label="弹出" onClick={() => router.pop()} />
                <Button wide label="对话框" onClick={() => setDialog(true)} />
              </row>
              <Dialog open={dialog()} onClose={() => setDialog(false)}>
                <decorated color={theme().color.surface} radius={theme().shape.l}>
                  <padding edges={theme().space.l}>
                    <column gap={theme().space.l} w="100%">
                      <Text
                        fontSize={theme().typography.title.size}
                        color={theme().color.text}
                        weight={theme().typography.title.weight}
                        direction={directionFor(theme().locale)}
                      >
                        返回键关闭
                      </Text>
                      <Button wide variant="primary" label="关闭" onClick={() => setDialog(false)} />
                    </column>
                  </padding>
                </decorated>
              </Dialog>
            </column>
          );
        },
      },
      { path: "/list", component: ListPage },
    ],
    { initial: "/schedule" },
  );
  return (
    <OverlayRoot>
      <Screen>
        <TabBar router={router} />
        <stack fill>
          <RouterView
            router={router}
            background={theme().color.background}
            errorFallback={(err, reset) => (
              <ErrorPage
                message={messageOf(typeof err === "function" ? err() : err)}
                stack={stackOf(typeof err === "function" ? err() : err)}
                retry={() => reset()}
                onBack={() => router.pop()}
              />
            )}
          />
        </stack>
      </Screen>
    </OverlayRoot>
  );
}

boot(App);
