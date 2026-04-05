// High-level WFE API for Deno.
//
// Usage:
//   import { WorkflowHost, WorkflowBuilder, ExecutionResult } from "ext:wfe-deno/wfe.js";

/// Factory methods for step execution results.
export class ExecutionResult {
    /** Proceed to the next step. */
    static next() {
        return { proceed: true };
    }

    /** Proceed with a routing outcome value. */
    static outcome(value) {
        return { proceed: true, outcomeValue: value };
    }

    /** Pause and persist step state for later resumption. */
    static persist(data) {
        return { proceed: false, persistenceData: data };
    }

    /** Sleep for the given number of milliseconds. */
    static sleep(ms, persistenceData) {
        const r = { proceed: false, sleepFor: ms };
        if (persistenceData !== undefined) r.persistenceData = persistenceData;
        return r;
    }

    /** Wait for an external event before continuing. */
    static waitForEvent(eventName, eventKey) {
        return { proceed: false, eventName, eventKey };
    }

    /** Fork into parallel branches. */
    static branch(values) {
        return { proceed: false, branchValues: values };
    }

    /** Proceed and attach output data. */
    static output(data) {
        return { proceed: true, outputData: data };
    }
}

/// Workflow engine host.
export class WorkflowHost {
    /** Create a WorkflowHost with in-memory providers. */
    static create() {
        Deno.core.ops.op_host_create();
        return new WorkflowHost();
    }

    /** Start the background workflow and event consumer loops. */
    async start() {
        await Deno.core.ops.op_host_start();
    }

    /** Stop the host gracefully. */
    async stop() {
        await Deno.core.ops.op_host_stop();
    }

    /**
     * Register a JavaScript function as a workflow step.
     *
     * @param {string} stepType - The step type name (used in builder).
     * @param {function} fn - Async function receiving context, returning ExecutionResult.
     */
    async registerStep(stepType, fn) {
        // Start the executor loop on first step registration.
        if (!this._executorStarted) {
            globalThis.__wfe_startExecutorLoop();
            this._executorStarted = true;
        }
        await Deno.core.ops.op_register_step(stepType);
        globalThis.__wfe_registerStepFunction(stepType, fn);
    }

    /**
     * Start a new workflow instance.
     *
     * @param {string} definitionId - The workflow definition ID.
     * @param {number} version - The workflow version.
     * @param {object} data - Initial workflow data.
     * @returns {Promise<string>} The workflow instance ID.
     */
    async startWorkflow(definitionId, version, data) {
        return await Deno.core.ops.op_start_workflow(definitionId, version, data);
    }

    /** Suspend a running workflow. */
    async suspendWorkflow(id) {
        return await Deno.core.ops.op_suspend_workflow(id);
    }

    /** Resume a suspended workflow. */
    async resumeWorkflow(id) {
        return await Deno.core.ops.op_resume_workflow(id);
    }

    /** Terminate a workflow. */
    async terminateWorkflow(id) {
        return await Deno.core.ops.op_terminate_workflow(id);
    }

    /** Get a workflow instance by ID. */
    async getWorkflow(id) {
        return await Deno.core.ops.op_get_workflow(id);
    }

    /** Publish an event for waiting workflows. */
    async publishEvent(eventName, eventKey, data) {
        await Deno.core.ops.op_publish_event(eventName, eventKey, data);
    }

    /**
     * Build and register a workflow definition using a fluent builder.
     *
     * @param {string} id - Workflow definition ID.
     * @param {number} version - Workflow version.
     * @param {function} builderFn - Function receiving a WorkflowBuilder.
     */
    async buildWorkflow(id, version, builderFn) {
        const b = new WorkflowBuilder();
        builderFn(b);
        await b._register(id, version);
    }
}

/// Fluent workflow builder wrapping the Rust WorkflowBuilder via ops.
export class WorkflowBuilder {
    constructor() {
        this._handle = Deno.core.ops.op_builder_create();
    }

    /** Add the first step. */
    startWith(stepType) {
        Deno.core.ops.op_builder_start_with(this._handle, stepType);
        return this;
    }

    /** Chain the next step. */
    then(stepType) {
        Deno.core.ops.op_builder_then(this._handle, stepType);
        return this;
    }

    /** Set the current step's display name. */
    name(n) {
        Deno.core.ops.op_builder_name(this._handle, n);
        return this;
    }

    /** Attach JSON configuration to the current step. */
    config(c) {
        Deno.core.ops.op_builder_config(this._handle, c);
        return this;
    }

    /** Set error handling behavior for the current step. */
    onError(behavior) {
        Deno.core.ops.op_builder_on_error(this._handle, behavior);
        return this;
    }

    /** Add a delay step (milliseconds). */
    delay(ms) {
        Deno.core.ops.op_builder_delay(this._handle, ms);
        return this;
    }

    /** Add a wait-for-event step. */
    waitFor(eventName, eventKey) {
        Deno.core.ops.op_builder_wait_for(this._handle, eventName, eventKey);
        return this;
    }

    /** Build and return the workflow definition as JSON. */
    build(id, version) {
        return Deno.core.ops.op_builder_build(this._handle, id, version);
    }

    /** Build and register the definition with the host. */
    async _register(id, version) {
        await Deno.core.ops.op_builder_register(this._handle, id, version);
    }
}
