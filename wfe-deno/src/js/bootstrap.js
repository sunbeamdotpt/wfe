// Import wfe.js so it's evaluated and available for user code.
import "ext:wfe-deno/wfe.js";

// Step function registry -- maps step type names to JS functions.
const _stepFunctions = {};

// Called by WorkflowHost.registerStep() to store the JS function.
globalThis.__wfe_registerStepFunction = (stepType, fn) => {
    _stepFunctions[stepType] = fn;
};

// Step executor loop -- runs in the background, dispatching step execution
// requests from the Rust executor to registered JS functions.
//
// This loop starts automatically when the extension loads. It blocks on
// op_step_executor_poll (async op) until a step needs to execute, then
// calls the matching JS function and sends the result back.
globalThis.__wfe_startExecutorLoop = () => {
    (async () => {
        while (true) {
            let req;
            try {
                req = await Deno.core.ops.op_step_executor_poll();
            } catch (_) {
                // Poll op failed (e.g. already active) -- stop loop.
                break;
            }
            if (req === null || req === undefined) break; // shutdown signal

            try {
                const fn = _stepFunctions[req.stepType];
                if (!fn) {
                    throw new Error(`No step function registered for "${req.stepType}"`);
                }
                const result = await fn(req.context);
                Deno.core.ops.op_step_executor_respond(
                    req.requestId,
                    result || { proceed: true },
                    null,
                );
            } catch (e) {
                Deno.core.ops.op_step_executor_respond(
                    req.requestId,
                    null,
                    e.message || String(e),
                );
            }
        }
    })();
};
