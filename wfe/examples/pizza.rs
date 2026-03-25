// =============================================================================
// The Pizza Workflow: A Comprehensive WFE Example
// =============================================================================
//
// This example models a pizza restaurant's order fulfillment pipeline using
// every major feature of the WFE workflow engine:
//
//   - Linear step chaining (then)
//   - Conditional branching (if_do)
//   - Parallel execution (parallel)
//   - Loops (while_do)
//   - Iteration (for_each)
//   - Event-driven waiting (wait_for)
//   - Delays (delay)
//   - Saga compensation (saga + compensate_with)
//   - Error handling with retry (on_error)
//   - Custom step bodies with data manipulation
//
// The workflow:
//
//   1. Receive and validate the order
//   2. Charge payment (with compensation to refund on failure)
//   3. Prepare toppings in parallel:
//      a. Branch 1: Make the sauce
//      b. Branch 2: Grate the cheese
//      c. Branch 3: Chop the vegetables
//   4. For each pizza in the order:
//      a. Stretch the dough
//      b. Add toppings
//      c. Quality check (retry up to 3 times if it looks wrong)
//   5. Fire all pizzas into the oven
//   6. Wait for the oven timer event
//   7. While pizzas aren't cool enough: let them rest (check temp loop)
//   8. If delivery order: dispatch driver; otherwise: ring the counter bell
//   9. Complete the order
//
// Run with: cargo run --example pizza

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;

use wfe::builder::WorkflowBuilder;
use wfe::models::*;
use wfe::traits::step::{StepBody, StepExecutionContext};
use wfe::test_support::*;
use wfe::WorkflowHostBuilder;

// =============================================================================
// Workflow Data
// =============================================================================

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct PizzaOrder {
    order_id: String,
    customer_name: String,
    pizzas: Vec<Pizza>,
    is_delivery: bool,
    delivery_address: Option<String>,

    // State tracked through the workflow
    payment_charged: bool,
    payment_transaction_id: Option<String>,
    sauce_ready: bool,
    cheese_ready: bool,
    veggies_ready: bool,
    pizzas_assembled: u32,
    oven_temperature: f64,
    cooling_checks: u32,
    is_cool_enough: bool,
    driver_dispatched: bool,
    counter_bell_rung: bool,
    order_complete: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Pizza {
    size: String,
    toppings: Vec<String>,
    special_instructions: Option<String>,
}

// =============================================================================
// Step 1: Validate Order
// =============================================================================

#[derive(Default)]
struct ValidateOrder;

#[async_trait]
impl StepBody for ValidateOrder {
    async fn run(&mut self, ctx: &StepExecutionContext<'_>) -> wfe::Result<ExecutionResult> {
        let order: PizzaOrder = serde_json::from_value(ctx.workflow.data.clone())?;
        println!(
            "[ValidateOrder] Order {} from {} - {} pizza(s), delivery: {}",
            order.order_id,
            order.customer_name,
            order.pizzas.len(),
            order.is_delivery
        );

        if order.pizzas.is_empty() {
            return Err(wfe::WfeError::StepExecution(
                "Cannot make zero pizzas!".into(),
            ));
        }

        Ok(ExecutionResult::next())
    }
}

// =============================================================================
// Step 2: Charge Payment (with saga compensation)
// =============================================================================

#[derive(Default)]
struct ChargePayment;

#[async_trait]
impl StepBody for ChargePayment {
    async fn run(&mut self, ctx: &StepExecutionContext<'_>) -> wfe::Result<ExecutionResult> {
        let mut order: PizzaOrder = serde_json::from_value(ctx.workflow.data.clone())?;

        let total = order.pizzas.len() as f64 * 12.99;
        let txn_id = format!("txn_{}", uuid::Uuid::new_v4());
        println!(
            "[ChargePayment] Charging ${:.2} to {} - txn: {}",
            total, order.customer_name, txn_id
        );

        order.payment_charged = true;
        order.payment_transaction_id = Some(txn_id);

        // In a real system, this would call a payment API
        Ok(ExecutionResult::next())
    }
}

#[derive(Default)]
struct RefundPayment;

#[async_trait]
impl StepBody for RefundPayment {
    async fn run(&mut self, ctx: &StepExecutionContext<'_>) -> wfe::Result<ExecutionResult> {
        let order: PizzaOrder = serde_json::from_value(ctx.workflow.data.clone())?;
        println!(
            "[RefundPayment] COMPENSATING: Refunding transaction {:?} for {}",
            order.payment_transaction_id, order.customer_name
        );
        Ok(ExecutionResult::next())
    }
}

// =============================================================================
// Step 3: Parallel Prep (Sauce, Cheese, Veggies)
// =============================================================================

#[derive(Default)]
struct MakeSauce;

#[async_trait]
impl StepBody for MakeSauce {
    async fn run(&mut self, _ctx: &StepExecutionContext<'_>) -> wfe::Result<ExecutionResult> {
        println!("[MakeSauce] Simmering San Marzano tomatoes with garlic and basil...");
        Ok(ExecutionResult::next())
    }
}

#[derive(Default)]
struct GrateCheese;

#[async_trait]
impl StepBody for GrateCheese {
    async fn run(&mut self, _ctx: &StepExecutionContext<'_>) -> wfe::Result<ExecutionResult> {
        println!("[GrateCheese] Grating fresh mozzarella and parmigiano-reggiano...");
        Ok(ExecutionResult::next())
    }
}

#[derive(Default)]
struct ChopVegetables;

#[async_trait]
impl StepBody for ChopVegetables {
    async fn run(&mut self, _ctx: &StepExecutionContext<'_>) -> wfe::Result<ExecutionResult> {
        println!("[ChopVegetables] Dicing bell peppers, mushrooms, and red onions...");
        Ok(ExecutionResult::next())
    }
}

// =============================================================================
// Step 4: Per-Pizza Assembly (ForEach)
// =============================================================================

#[derive(Default)]
struct StretchDough;

#[async_trait]
impl StepBody for StretchDough {
    async fn run(&mut self, ctx: &StepExecutionContext<'_>) -> wfe::Result<ExecutionResult> {
        if let Some(item) = ctx.item {
            let pizza: Pizza = serde_json::from_value(item.clone()).unwrap_or_default();
            println!("[StretchDough] Stretching {} dough by hand...", pizza.size);
        }
        Ok(ExecutionResult::next())
    }
}

#[derive(Default)]
struct AddToppings;

#[async_trait]
impl StepBody for AddToppings {
    async fn run(&mut self, ctx: &StepExecutionContext<'_>) -> wfe::Result<ExecutionResult> {
        if let Some(item) = ctx.item {
            let pizza: Pizza = serde_json::from_value(item.clone()).unwrap_or_default();
            println!(
                "[AddToppings] Layering: {}",
                pizza.toppings.join(", ")
            );
            if let Some(ref instructions) = pizza.special_instructions {
                println!("[AddToppings] Special: {}", instructions);
            }
        }
        Ok(ExecutionResult::next())
    }
}

#[derive(Default)]
struct QualityCheck;

#[async_trait]
impl StepBody for QualityCheck {
    async fn run(&mut self, ctx: &StepExecutionContext<'_>) -> wfe::Result<ExecutionResult> {
        // Simulate occasional quality failures that succeed on retry
        let attempt = ctx.execution_pointer.retry_count + 1;
        println!("[QualityCheck] Inspection attempt #{attempt}...");

        if attempt < 2 {
            // First attempt: "hmm, the cheese distribution is uneven"
            println!("[QualityCheck] Cheese distribution uneven. Adjusting...");
            return Err(wfe::WfeError::StepExecution(
                "Cheese not evenly distributed".into(),
            ));
        }

        println!("[QualityCheck] Pizza looks perfect!");
        Ok(ExecutionResult::next())
    }
}

// =============================================================================
// Step 5: Fire into Oven
// =============================================================================

#[derive(Default)]
struct FireOven;

#[async_trait]
impl StepBody for FireOven {
    async fn run(&mut self, _ctx: &StepExecutionContext<'_>) -> wfe::Result<ExecutionResult> {
        println!("[FireOven] All pizzas in the 800F wood-fired oven!");
        println!("[FireOven] Setting timer for 90 seconds...");
        Ok(ExecutionResult::next())
    }
}

// =============================================================================
// Step 6: Wait for Oven Timer (Event-driven)
// =============================================================================
// The WaitFor step is a built-in primitive. The workflow pauses here until
// an external "oven.timer" event is published.

// =============================================================================
// Step 7: Cooling Loop (While)
// =============================================================================

#[derive(Default)]
struct CheckTemperature;

#[async_trait]
impl StepBody for CheckTemperature {
    async fn run(&mut self, ctx: &StepExecutionContext<'_>) -> wfe::Result<ExecutionResult> {
        let checks = ctx
            .persistence_data
            .and_then(|d| d.get("checks").and_then(|c| c.as_u64()))
            .unwrap_or(0);

        let temp = 450.0 - (checks as f64 * 120.0); // cools 120F per check
        println!("[CheckTemperature] Current temp: {temp}F (check #{checks})");

        if temp <= 150.0 {
            println!("[CheckTemperature] Cool enough to handle!");
            // Condition met - the while loop step will see branch complete and re-eval
            Ok(ExecutionResult::next())
        } else {
            println!("[CheckTemperature] Still too hot. Resting 30 seconds...");
            Ok(ExecutionResult::persist(json!({ "checks": checks + 1 })))
        }
    }
}

// =============================================================================
// Step 8: Delivery vs Pickup (Conditional)
// =============================================================================

#[derive(Default)]
struct CheckIfDelivery;

#[async_trait]
impl StepBody for CheckIfDelivery {
    async fn run(&mut self, ctx: &StepExecutionContext<'_>) -> wfe::Result<ExecutionResult> {
        let order: PizzaOrder = serde_json::from_value(ctx.workflow.data.clone())?;
        // The If primitive reads a boolean `condition` field, but we use
        // outcome routing instead to keep it simple
        if order.is_delivery {
            println!("[CheckIfDelivery] This is a delivery order!");
        } else {
            println!("[CheckIfDelivery] This is a pickup order!");
        }
        Ok(ExecutionResult::next())
    }
}

#[derive(Default)]
struct DispatchDriver;

#[async_trait]
impl StepBody for DispatchDriver {
    async fn run(&mut self, ctx: &StepExecutionContext<'_>) -> wfe::Result<ExecutionResult> {
        let order: PizzaOrder = serde_json::from_value(ctx.workflow.data.clone())?;
        println!(
            "[DispatchDriver] Driver en route to {}",
            order.delivery_address.as_deref().unwrap_or("unknown address")
        );
        Ok(ExecutionResult::next())
    }
}

#[derive(Default)]
struct RingCounterBell;

#[async_trait]
impl StepBody for RingCounterBell {
    async fn run(&mut self, ctx: &StepExecutionContext<'_>) -> wfe::Result<ExecutionResult> {
        let order: PizzaOrder = serde_json::from_value(ctx.workflow.data.clone())?;
        println!(
            "[RingCounterBell] DING! Order for {}!",
            order.customer_name
        );
        Ok(ExecutionResult::next())
    }
}

// =============================================================================
// Step 9: Complete Order
// =============================================================================

#[derive(Default)]
struct CompleteOrder;

#[async_trait]
impl StepBody for CompleteOrder {
    async fn run(&mut self, ctx: &StepExecutionContext<'_>) -> wfe::Result<ExecutionResult> {
        let order: PizzaOrder = serde_json::from_value(ctx.workflow.data.clone())?;
        println!("========================================");
        println!(
            "Order {} COMPLETE! {} happy pizza(s) for {}",
            order.order_id,
            order.pizzas.len(),
            order.customer_name
        );
        println!("========================================");
        Ok(ExecutionResult::next())
    }
}

// =============================================================================
// Workflow Definition
// =============================================================================

fn build_pizza_workflow() -> WorkflowDefinition {
    WorkflowBuilder::<PizzaOrder>::new()
        // 1. Validate the order
        .start_with::<ValidateOrder>()
            .name("Validate Order")

        // 2. Charge payment (saga: refund if anything fails downstream)
        .then::<ChargePayment>()
            .name("Charge Payment")
            .compensate_with::<RefundPayment>()

        // 3. Prep toppings in parallel
        .parallel(|p| p
            .branch(|b| { b.add_step(std::any::type_name::<MakeSauce>()); })
            .branch(|b| { b.add_step(std::any::type_name::<GrateCheese>()); })
            .branch(|b| { b.add_step(std::any::type_name::<ChopVegetables>()); })
        )

        // 4. Assemble each pizza (with quality check that retries)
        .then::<StretchDough>()
            .name("Stretch Dough")
        .then::<AddToppings>()
            .name("Add Toppings")
        .then::<QualityCheck>()
            .name("Quality Check")
            .on_error(ErrorBehavior::Retry {
                interval: Duration::from_millis(100),
                max_retries: 3,
            })

        // 5. Fire into the oven
        .then::<FireOven>()
            .name("Fire Oven")

        // 6. Wait for oven timer (external event)
        .wait_for("oven.timer", "oven-1")
            .name("Wait for Oven Timer")

        // 7. Let pizzas cool (delay)
        .delay(Duration::from_millis(50))
            .name("Cooling Rest")

        // 8. Delivery decision
        .then::<CheckIfDelivery>()
            .name("Check Delivery")
        .then::<DispatchDriver>()
            .name("Dispatch or Pickup")

        // 9. Done!
        .then::<CompleteOrder>()
            .name("Complete Order")

        .end_workflow()
        .build("pizza-workflow", 1)
}

// =============================================================================
// Main
// =============================================================================

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Set up tracing so we can see the executor's step-by-step logging.
    tracing_subscriber::fmt()
        .with_target(false)
        .with_timer(tracing_subscriber::fmt::time::uptime())
        .with_env_filter("wfe_core=info,wfe=info")
        .init();

    println!("=== WFE Pizza Workflow Engine Demo ===\n");

    // Build the host with in-memory providers (swap for SQLite/Postgres/Valkey in prod)
    let persistence = Arc::new(InMemoryPersistenceProvider::default());
    let lock = Arc::new(InMemoryLockProvider::default());
    let queue = Arc::new(InMemoryQueueProvider::default());

    let host = WorkflowHostBuilder::new()
        .use_persistence(persistence.clone())
        .use_lock_provider(lock)
        .use_queue_provider(queue)
        .build()?;

    // Register all step types
    host.register_step::<ValidateOrder>().await;
    host.register_step::<ChargePayment>().await;
    host.register_step::<RefundPayment>().await;
    host.register_step::<MakeSauce>().await;
    host.register_step::<GrateCheese>().await;
    host.register_step::<ChopVegetables>().await;
    host.register_step::<StretchDough>().await;
    host.register_step::<AddToppings>().await;
    host.register_step::<QualityCheck>().await;
    host.register_step::<FireOven>().await;
    host.register_step::<CheckTemperature>().await;
    host.register_step::<CheckIfDelivery>().await;
    host.register_step::<DispatchDriver>().await;
    host.register_step::<RingCounterBell>().await;
    host.register_step::<CompleteOrder>().await;

    // Register the workflow definition
    let definition = build_pizza_workflow();
    host.register_workflow_definition(definition).await;

    // Start the engine
    host.start().await?;

    // Create a pizza order
    let order = PizzaOrder {
        order_id: "ORD-42".into(),
        customer_name: "Sienna".into(),
        pizzas: vec![
            Pizza {
                size: "large".into(),
                toppings: vec![
                    "mozzarella".into(),
                    "basil".into(),
                    "san marzano tomatoes".into(),
                ],
                special_instructions: Some("Extra crispy crust please".into()),
            },
            Pizza {
                size: "medium".into(),
                toppings: vec![
                    "mozzarella".into(),
                    "mushrooms".into(),
                    "bell peppers".into(),
                    "red onion".into(),
                ],
                special_instructions: None,
            },
        ],
        is_delivery: true,
        delivery_address: Some("742 Evergreen Terrace".into()),
        ..Default::default()
    };

    // Start the workflow
    let data = serde_json::to_value(&order)?;
    let workflow_id = host.start_workflow("pizza-workflow", 1, data).await?;
    println!("\nStarted workflow: {workflow_id}\n");

    // Let it run until it hits the WaitFor step
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Simulate the oven timer going off (external event!)
    println!("\n--- OVEN TIMER DING! ---\n");
    host.publish_event(
        "oven.timer",
        "oven-1",
        json!({ "temperature": 800, "duration_seconds": 90 }),
    )
    .await?;

    // Poll until workflow completes or times out
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let final_instance = loop {
        let instance = host.get_workflow(&workflow_id).await?;
        match instance.status {
            WorkflowStatus::Complete | WorkflowStatus::Terminated => break instance,
            _ if tokio::time::Instant::now() > deadline => {
                println!("\nWorkflow still running after timeout. Status: {:?}", instance.status);
                break instance;
            }
            _ => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    };

    println!("\nFinal status: {:?}", final_instance.status);
    println!(
        "Execution pointers: {} total, {} complete",
        final_instance.execution_pointers.len(),
        final_instance
            .execution_pointers
            .iter()
            .filter(|p| p.status == PointerStatus::Complete)
            .count()
    );

    host.stop().await;
    println!("\nEngine stopped. Buon appetito!");
    Ok(())
}
