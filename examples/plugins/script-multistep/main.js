export async function run(ctx, input) {
  const response = JSON.parse(
    await ctx.http.request("GET", "https://api.example.com/v1/context")
  );
  const answer = await ctx.ai.invoke(
    `User text:\n${input.text}\n\nExternal context:\n${response.body}`,
    "Summarize the supplied user text using only relevant external context."
  );
  await ctx.storage.set("last-run", JSON.stringify({ status: response.status }));
  ctx.output.append(answer);
  return { type: "complete" };
}
