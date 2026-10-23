import { createSignal } from "solid-js";
import { Text, boot, onFrame, requestFrame } from "@uifw/solid-renderer";
import { Button, Field, ScrollArea } from "@uifw/solid-ui";

function Counter() {
  const [count, setCount] = createSignal(0);
  const [alpha, setAlpha] = createSignal(255);
  console.log("boot", count());
  onFrame((timeMs) => {
    setAlpha(128 + Math.round(127 * Math.sin(timeMs / 300)));
    requestFrame();
  });
  requestFrame();
  return (
    <column gap={8}>
      <opacity alpha={alpha()}>
        <Text fontSize={16} color="#FFFFFF">
          Count: {count()}
        </Text>
      </opacity>
      <image source="asset:logo" w={56} h={56} />
      <row gap={8}>
        <Button label="-" onClick={() => setCount(count() - 1)} />
        <Button label="+" onClick={() => setCount(count() + 1)} />
      </row>
    </column>
  );
}

function ListPage() {
  const rows = Array.from({ length: 50 }, (_, index) => index);
  return (
    <ScrollArea>
      {rows.map((index) => (
        <row gap={8}>
          <Text fontSize={16} color="#FFFFFF">
            {`Item ${index}`}
          </Text>
          <Button label="Open" onClick={() => {}} />
        </row>
      ))}
    </ScrollArea>
  );
}

function FormPage() {
  const [name, setName] = createSignal("");
  const [note, setNote] = createSignal("");
  const [submitted, setSubmitted] = createSignal("");
  return (
    <ScrollArea>
      <Field placeholder="Name" onInput={setName} />
      <Field placeholder="Note" onInput={setNote} />
      <Button label="Submit" onClick={() => setSubmitted(`${name()} ${note()}`)} />
      <Text fontSize={16} color="#FFFFFF">
        {submitted()}
      </Text>
    </ScrollArea>
  );
}

function App() {
  return (
    <column gap={8} fill>
      <Counter />
      <ListPage />
      <FormPage />
    </column>
  );
}

boot(App);
