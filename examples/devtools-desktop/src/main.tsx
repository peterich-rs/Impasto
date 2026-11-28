import { createMemo } from "solid-js";
import { boot } from "@impasto/solid-renderer";
import { OverlayRoot, useTheme } from "@impasto/solid-ui";
import { Workbench } from "./shell/Workbench";
import { createWorkspace, Workspace } from "./state/workspace";
import { palette, Tokens } from "./theme/tokens";

function Desktop() {
  const system = useTheme();
  const workspace = createWorkspace();
  const tokens = createMemo(() => {
    const selected = workspace.theme();
    return palette(selected === "system" ? system().brightness : selected);
  });
  return <Workspace value={workspace}>
    <Tokens value={tokens}>
      <OverlayRoot><Workbench /></OverlayRoot>
    </Tokens>
  </Workspace>;
}

boot(Desktop);
