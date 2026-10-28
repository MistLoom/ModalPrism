export function register(on) {
  on("session.start", async ($, e, next) => {
    await next(e);
    await $.session.usage();
  });

  on("turn.complete", async ($, e, next) => {
    await next(e);
    await $.session.usage();
  });

  on("ui.render", { component: "AbovePrompt" }, ($, e, next) => {
    return next(e);
  });
}
