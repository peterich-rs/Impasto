import { createSignal, onCleanup, Show } from "solid-js";
import { AsyncStorage } from "@impasto/async-storage";
import { Restoration } from "@impasto/app-state";
import { availableFormats } from "@impasto/clipboard";
import { getStatus } from "@impasto/connectivity";
import { FileSystem } from "@impasto/file-system";
import { impact } from "@impasto/haptics";
import { Linking } from "@impasto/linking";
import { Permission, type PermissionHandle } from "@impasto/permissions";
import { shareFiles, shareText } from "@impasto/share";
import { boot } from "@impasto/solid-renderer";
import { createRouter, RouterView, type RouteParams } from "@impasto/solid-router";
import { Button, Column, ExternalTexture, Field, KeyboardAvoidingView, PlatformView, ScrollArea, Text } from "@impasto/solid-ui";
import { getAppInfo } from "@impasto/system-info";

type Draft = { version: 1; record: string; note: string; attachment: string };

const draftKey = "field-notes.draft";
const routeKey = "field-notes.routes";
const paper = "#F4EFE5";
const ink = "#252F2A";
const muted = "#536358";
const page = { padding: 24, gap: 16 } as const;
const button = { borderRadius: 12 } as const;
const primary = { ...button, backgroundColor: "#2E5A47" } as const;
const primaryLabel = { style: { color: "#FFFFFF" } } as const;

function recordFromUrl(url: string | null) {
  return url?.match(/^impasto-demo:\/\/record\/([a-zA-Z0-9-]+)(?:[?#].*)?$/)?.[1] ??
    url?.match(/^https:\/\/[^/?#]+\/records\/([a-zA-Z0-9-]+)(?:[?#].*)?$/)?.[1];
}

function deserializeDraft(value: string | null): Draft | null {
  if (!value) return null;
  let draft: Partial<Draft> | null;
  try {
    draft = JSON.parse(value);
  } catch {
    return null;
  }
  if (draft?.version !== 1 || typeof draft.record !== "string" ||
      !/^[a-zA-Z0-9-]+$/.test(draft.record) || typeof draft.note !== "string" ||
      typeof draft.attachment !== "string") return null;
  return { version: 1, record: draft.record, note: draft.note, attachment: draft.attachment };
}

export default function Main() {
  const [draft, setDraft] = createSignal<Draft | null>(null);
  const [ready, setReady] = createSignal(false);
  const hasTaskRoutes = Restoration.get(routeKey) !== null;
  const initialUrl = Linking.getInitialURL();
  let pendingRecord = recordFromUrl(initialUrl);
  const nativeHost = initialUrl?.endsWith("?native=1") === true ||
    Restoration.get("field-notes.native-host") === "1";
  if (nativeHost) Restoration.set("field-notes.native-host", "1");

  function openRecord(record: string) {
    const path = `/records/${record}`;
    if (router.stack().at(-1)?.path === path) return;
    if (router.depth() > 1) router.replace(path);
    else router.push(path);
  }

  function Home() {
    return (
      <Column style={page}>
        <Text style={{ fontSize: 34, fontWeight: 600, color: ink }}>现场记录</Text>
        <Text style={{ fontSize: 16, color: muted }}>记录一次现场检查，保存备注和相关文件。</Text>
        <Button label="打开记录 42" style={primary} labelProps={primaryLabel} onClick={() => openRecord("42")} />
      </Column>
    );
  }

  function Details(props: { params: RouteParams }) {
    const record = props.params.id ?? "42";
    const taskDraft = deserializeDraft(Restoration.get(draftKey));
    const saved = taskDraft?.record === record ? taskDraft : draft()?.record === record ? draft() : null;
    const [note, setNote] = createSignal(saved?.note ?? "");
    const [attachment, setAttachment] = createSignal(saved?.attachment ?? "");
    const [status, setStatus] = createSignal(saved ? "已恢复草稿" : "");
    const [saving, setSaving] = createSignal(false);
    const [picking, setPicking] = createSignal(false);
    const scrollKey = "field-notes.scroll";
    const scroll = Restoration.get(scrollKey)?.split(":");
    const savedScroll = scroll?.[0] === record ? Number(scroll[1]) : 0;
    let scrollY = Number.isFinite(savedScroll) && savedScroll > 0 ? savedScroll : 0;

    function remember(note: string, attachment: string) {
      if (note.length > 4096) {
        Restoration.set(draftKey, null);
      } else {
        Restoration.set(draftKey, JSON.stringify({ version: 1, record, note, attachment }));
      }
    }

    async function save() {
      const value: Draft = { version: 1, record, note: note(), attachment: attachment() };
      setSaving(true);
      try {
        await AsyncStorage.multiSet([[draftKey, JSON.stringify(value)]]);
        setDraft(value);
        setStatus("草稿已保存");
      } catch (error) {
        console.warn("草稿保存失败", error instanceof Error ? error.message : String(error));
        setStatus("未能保存草稿，请重试");
      } finally {
        setSaving(false);
      }
    }

    async function pick() {
      setPicking(true);
      try {
        const file = await FileSystem.pick();
        if (file) {
          setAttachment(file.name);
          remember(note(), file.name);
          setStatus("文件已添加");
        }
      } catch (error) {
        console.warn("文件选择失败", error instanceof Error ? error.message : String(error));
        setStatus("未能读取文件，请重新选择");
      } finally {
        setPicking(false);
      }
    }

    async function requestPermission(label: string, permission: PermissionHandle) {
      try {
        const access = await permission.request();
        setStatus(`${label}权限 · ${access}`);
      } catch (error) {
        setStatus(`${label}权限 · ${String(error)}`);
      }
    }

    async function share() {
      try {
        const text = `现场记录 ${record}\n${note()}`;
        const file = attachment();
        const result = file ? await shareFiles([{ path: file }], { text }) : await shareText(text);
        setStatus(`分享 · ${result.status}`);
      } catch {
        setStatus("未能打开系统分享");
      }
    }

    async function showStatus(action: () => Promise<string>) {
      try { setStatus(await action()); }
      catch (error) { setStatus(String(error)); }
    }

    return (
      <KeyboardAvoidingView behavior="padding" style={{ flex: 1, minHeight: 0 }}>
        <ScrollArea
          style={{ flex: 1, minHeight: 0 }}
          initialOffset={{ x: 0, y: scrollY }}
          onOffset={({ y }) => {
            const rounded = Math.round(y);
            if (rounded === scrollY) return;
            scrollY = rounded;
            // Keep only the current record's offset in the task bundle.
            Restoration.set(scrollKey, rounded > 0 ? `${record}:${rounded}` : null);
          }}
        >
          <Column style={page}>
            <Button label="返回记录列表" style={{ ...button, alignSelf: "flex-start" }} onClick={() => router.pop()} />
            <Text style={{ fontSize: 30, fontWeight: 600, color: ink }}>{`现场记录 ${record}`}</Text>
            <Field label="检查备注" placeholder="输入检查结果" value={note()} onChange={(next) => {
              setNote(next);
              remember(next, attachment());
            }} />
            <Show when={nativeHost}>
              <PlatformView kind="site-editor" style={{ width: "100%", height: 56 }} />
              <PlatformView kind="site-checklist" style={{ width: "100%", height: 88 }} />
              <ExternalTexture name="site-photo" alt="本次现场照片" intrinsicSize={{ width: 320, height: 180 }}
                style={{ width: "100%", height: 156 }} />
            </Show>
            <Show when={attachment()}>
              <Text style={{ fontSize: 15, color: muted }}>{() => attachment().split("/").at(-1)}</Text>
            </Show>
            <Button label="选择文件" style={button} disabled={picking()} onClick={() => void pick()} />
            <Button label="保存草稿" style={primary} labelProps={primaryLabel} disabled={saving()} onClick={() => void save()} />
            <Button label="请求相机权限" style={button} onClick={() => void requestPermission("相机", Permission.camera)} />
            <Button label="请求相册权限" style={button} onClick={() => void requestPermission("相册", Permission.photos)} />
            <Button label="请求定位权限" style={button} onClick={() => void requestPermission("定位", Permission.locationWhenInUse)} />
            <Button label="请求精确闹钟权限" style={button} onClick={() => void requestPermission("精确闹钟", Permission.scheduleExactAlarm)} />
            <Button label="分享记录" style={button} onClick={() => void share()} />
            <Button label="在应用内打开项目主页" style={button} onClick={() => void showStatus(async () =>
              (await Linking.openURL("https://github.com/peterich-rs/Impasto", { mode: "inAppBrowser" }))
                ? "已打开项目主页" : "未找到可用浏览器")} />
            <Button label="查看网络状态" style={button} onClick={() => void showStatus(async () =>
              JSON.stringify(await getStatus()))} />
            <Button label="查看应用信息" style={button} onClick={() => void showStatus(async () =>
              JSON.stringify(await getAppInfo()))} />
            <Button label="播放触感反馈" style={button} onClick={() => void showStatus(async () => {
              await impact("medium");
              return "已请求触感反馈";
            })} />
            <Button label="查看剪贴板格式" style={button} onClick={() => void showStatus(async () =>
              (await availableFormats()).join("、") || "剪贴板为空")} />
            <Show when={status()}>
              <Text style={{ fontSize: 15, color: muted }}>{status()}</Text>
            </Show>
          </Column>
        </ScrollArea>
      </KeyboardAvoidingView>
    );
  }

  const router = createRouter([
    { path: "/", component: Home },
    { path: "/records/:id", component: Details },
  ], { linking: false, restoration: { ...Restoration, key: routeKey } });
  const receiveLink = ({ url }: { url: string }) => {
    const record = recordFromUrl(url);
    if (record && ready()) openRecord(record);
    else if (record) pendingRecord = record;
  };
  Linking.addEventListener("url", receiveLink);
  onCleanup(() => Linking.removeEventListener("url", receiveLink));

  const restore = async () => {
    try {
      const [[, value]] = await AsyncStorage.multiGet([draftKey]);
      setDraft(deserializeDraft(value));
    } catch (error) {
      console.warn("草稿恢复失败", error instanceof Error ? error.message : String(error));
      setDraft(null);
    }
    const record = pendingRecord ?? (hasTaskRoutes ? undefined : draft()?.record);
    if (record) openRecord(record);
    setReady(true);
  };
  void restore();

  return (
    <Column style={{ flex: 1, backgroundColor: paper }}>
      <Show when={ready()} fallback={<Text style={{ color: muted }}>正在恢复记录</Text>}>
        <RouterView router={router} background={paper} opaque />
      </Show>
    </Column>
  );
}

boot(Main);
