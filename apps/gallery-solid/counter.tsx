import { Show, createSignal, onCleanup } from "solid-js";
import { Text, boot, frameListenerCount, host, onFrame } from "@uifw/solid-renderer";
import { Button, Dialog, Field, VirtualList } from "@uifw/solid-ui";

type Page = "schedule" | "nav" | "list";

function SchedulePage() {
  const [label, setLabel] = createSignal("还是旧的");
  return (
    <column gap={8}>
      <Text fontSize={16} color="#FFFFFF">
        {label()}
      </Text>
      <Button
        label="微任务改字"
        onClick={() => {
          queueMicrotask(() => setLabel("已经改了"));
        }}
      />
    </column>
  );
}

function NavPage() {
  const [depth, setDepth] = createSignal(0);
  const [dialog, setDialog] = createSignal(false);
  const [listeners, setListeners] = createSignal(frameListenerCount());
  onCleanup(onFrame(() => setListeners(frameListenerCount())));
  const back = host.onBack(() => {
    if (depth() === 0) {
      return false;
    }
    setDepth(depth() - 1);
    return true;
  });
  onCleanup(() => host.offBack(back));
  return (
    <column gap={8}>
      <Text fontSize={16} color="#FFFFFF">
        {`第 ${depth() + 1} 页`}
      </Text>
      <Text fontSize={16} color="#FFFFFF">
        {`监听 ${listeners()}`}
      </Text>
      <Button label="推进" onClick={() => setDepth(depth() + 1)} />
      <Button label="弹出" onClick={() => setDepth(Math.max(0, depth() - 1))} />
      <Button label="对话框" onClick={() => setDialog(true)} />
      <Dialog open={dialog()} onClose={() => setDialog(false)}>
        <Text fontSize={16} color="#FFFFFF">
          返回键关闭
        </Text>
      </Dialog>
    </column>
  );
}

function ListPage() {
  const [scale, setScale] = createSignal(1);
  const [missing, setMissing] = createSignal(false);
  const [name, setName] = createSignal("");
  return (
    <column gap={8} fill>
      <row gap={8}>
        <Button
          label={scale() === 1 ? "字号 2.0" : "字号 1.0"}
          onClick={() => {
            const next = scale() === 1 ? 2 : 1;
            setScale(next);
            host.setTextScale(next);
          }}
        />
        <Button label={missing() ? "去掉缺图" : "缺图"} onClick={() => setMissing(!missing())} />
      </row>
      <Show when={missing()}>
        <image source="asset:missing" w={32} h={32} />
      </Show>
      <Field placeholder="姓名" onChange={setName} />
      <Text fontSize={16} color="#FFFFFF">
        {name() || "未填写"}
      </Text>
      <VirtualList
        items={Array.from({ length: 200 }, (_, n) => ({ n }))}
        itemKey={(item) => item.n}
        estimatedHeight={28}
      >
        {(item) => (
          <Text fontSize={16} color="#FFFFFF">
            {`Item ${item.n}`}
          </Text>
        )}
      </VirtualList>
    </column>
  );
}

function App() {
  const [page, setPage] = createSignal<Page>("schedule");
  return (
    <column gap={8} fill>
      <row gap={8}>
        <Button label="调度" onClick={() => setPage("schedule")} />
        <Button label="导航" onClick={() => setPage("nav")} />
        <Button label="列表" onClick={() => setPage("list")} />
      </row>
      <Show when={page() === "schedule"}>
        <SchedulePage />
      </Show>
      <Show when={page() === "nav"}>
        <NavPage />
      </Show>
      <Show when={page() === "list"}>
        <ListPage />
      </Show>
    </column>
  );
}

boot(App);
