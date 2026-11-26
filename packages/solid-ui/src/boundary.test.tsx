import { assert, screen, test } from "impasto:test";
import { Errored, createSignal } from "solid-js";
import { Text } from "@impasto/solid-renderer";
import { View } from "@impasto/solid-renderer/host";

function Fault(props: { read: () => boolean }) {
  if (props.read()) throw new Error("boom");
  return <Text>healthy</Text>;
}

test("the error boundary replaces its content in the live scene", async () => {
  const [fault, setFault] = createSignal(true);
  await screen.mount(() => (
    <View><Errored fallback={() => <Text>fallback</Text>}>
      <Fault read={fault} />
    </Errored></View>
  ));
  assert.equal((await screen.nodes("text", "fallback")).length, 1);
  setFault(false);
  await screen.mount(() => (
    <View><Errored fallback={() => <Text>fallback</Text>}>
      <Fault read={fault} />
    </Errored></View>
  ));
  assert.equal((await screen.nodes("text", "healthy")).length, 1);
  assert.equal((await screen.nodes("text", "fallback")).length, 0);
});
