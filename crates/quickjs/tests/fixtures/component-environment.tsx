import { createRoot } from "solid-js";
import { boot, onFrame, scene } from "@impasto/solid-renderer";
import { Column, Row, Text, View, useTheme } from "@impasto/solid-ui";

const check = (condition: unknown, message: string) => { if (!condition) throw new Error(message); };
createRoot(() => {
  const theme = useTheme();
  let a = 0, b = 0, localA = 0, localB = 0, variant = 0;
  boot(() => <Column style={{ width: 240, height: 300 }}>
    <Row style={{ width: 40 }}>
      <View ref={id => { a = id ?? 0; }} style={{ width: 10, height: 10 }} />
      <View ref={id => { b = id ?? 0; }} style={{ width: 10, height: 10 }} />
    </Row>
    <Row style={{ width: 40, layoutDirection: "ltr" }}>
      <View ref={id => { localA = id ?? 0; }} style={{ width: 10, height: 10 }} />
      <View ref={id => { localB = id ?? 0; }} style={{ width: 10, height: 10 }} />
    </Row>
    <View role={theme().locale === "ar" ? undefined : 1}
      accessibilityLabel={theme().locale === "ar" ? undefined : "removable"}
      accessibilityHint={theme().locale === "ar" ? undefined : "old-hint"}
      style={{ width: 20, height: 20 }} />
    <Text class="text-body text-foreground">class-token</Text>
    <Text style={{ color: { token: "foreground" } }}>object-token</Text>
    <View class="w-4 dark:w-8" ref={id => { if (id) variant = id; }} />
  </Column>);
  onFrame(() => {
    const rtl = theme().locale === "ar";
    check((scene.rect(a).x > scene.rect(b).x) === rtl, "root locale direction is inherited");
    check(scene.rect(localA).x < scene.rect(localB).x, "local LTR overrides the root environment");
    check(scene.rect(variant).w === (theme().brightness === "dark" ? 32 : 16), "native dark variant follows theme");
  });
});
