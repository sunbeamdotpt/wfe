/// Builder.
pub mod builder;
/// Event.
pub mod event;
/// Host.
pub mod host;
/// Step.
pub mod step;
/// Workflow.
pub mod workflow;

deno_core::extension!(
    wfe_deno_ext,
    ops = [
        // Host lifecycle
        host::op_host_create,
        host::op_host_start,
        host::op_host_stop,
        // Workflow management
        workflow::op_start_workflow,
        workflow::op_suspend_workflow,
        workflow::op_resume_workflow,
        workflow::op_terminate_workflow,
        workflow::op_get_workflow,
        // Builder
        builder::op_builder_create,
        builder::op_builder_start_with,
        builder::op_builder_then,
        builder::op_builder_name,
        builder::op_builder_config,
        builder::op_builder_on_error,
        builder::op_builder_delay,
        builder::op_builder_wait_for,
        builder::op_builder_build,
        builder::op_builder_register,
        // Step execution bridge
        step::op_register_step,
        step::op_step_executor_poll,
        step::op_step_executor_respond,
        // Events
        event::op_publish_event,
    ],
    esm_entry_point = "ext:wfe-deno/bootstrap.js",
    esm = [
        "ext:wfe-deno/bootstrap.js" = "src/js/bootstrap.js",
        "ext:wfe-deno/wfe.js" = "src/js/wfe.js",
    ],
);
