globalThis.inputs = () => Deno.core.ops.op_inputs();
globalThis.output = (key, value) => Deno.core.ops.op_output(key, value);
globalThis.log = (msg) => Deno.core.ops.op_log(msg);
