export async function invoke(
  _identifier: string,
  _method: string,
  _payload: string,
): Promise<string> {
  throw new Error("Native plugins require an iOS host");
}
