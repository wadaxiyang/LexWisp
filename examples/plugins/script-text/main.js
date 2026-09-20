import { transform } from "./modules/text.js";

export async function run(_ctx, input, params) {
  return { type: "text", text: transform(input.text.trim(), params.uppercase) };
}
