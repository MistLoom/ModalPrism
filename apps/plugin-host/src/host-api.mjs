export function createHostApi(hostCall) {
  const ns = (namespace) =>
    new Proxy(
      {},
      {
        get(_t, method) {
          return (...args) => hostCall({ ns: String(namespace), method: String(method), args });
        },
      },
    );
  return new Proxy(
    {},
    {
      get(_t, prop) {
        return ns(prop);
      },
    },
  );
}
