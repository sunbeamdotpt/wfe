globalThis.inputs = () => Deno.core.ops.op_inputs();
globalThis.output = (key, value) => Deno.core.ops.op_output(key, value);
globalThis.log = (msg) => Deno.core.ops.op_log(msg);

globalThis.fetch = async (url, options) => {
    const resp = await Deno.core.ops.op_fetch(url, options || null);
    return {
        status: resp.status,
        ok: resp.ok,
        headers: resp.headers,
        text: () => Promise.resolve(resp.body),
        json: () => Promise.resolve(JSON.parse(resp.body)),
    };
};
