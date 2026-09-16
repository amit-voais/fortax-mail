//! The assistant dialog's callbacks.
//!
//! Four verbs, kept apart on purpose. Answering and summarising only read. Drafting writes into the
//! composer, where a person still presses Send. Planning proposes a rule and stops; saving it is a
//! second, separate act, and one the agent level has to allow. Nothing here changes the mailbox.

use super::*;

/// The thread the person is looking at, or nothing when the list has no selection.
fn selected_thread(state: &Rc<RefCell<InboxState>>) -> Option<i64> {
    let state = state.borrow();
    let selected = state
        .selected_id
        .and_then(|selected_id| state.messages.iter().find(|m| m.id == selected_id))?;
    selected.thread_id
}

pub(super) fn register_assistant_callbacks(
    app: &AppWindow,
    state: &Rc<RefCell<InboxState>>,
    runtime: &Rc<tokio::runtime::Runtime>,
) {
    let app_weak = app.as_weak();
    let ask_state = Rc::clone(state);
    let ask_runtime = Rc::clone(runtime);
    app.on_assistant_ask(move |question| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let Some(core) = ask_state.borrow().core.clone() else {
            return;
        };
        let question = question.trim().to_string();
        if question.is_empty() {
            return;
        }
        app.set_assistant_busy(true);
        // `ai_ask` searches the mailbox itself, so this is the one verb that reaches past the open
        // thread. The scope setting is what the person used to allow that.
        let result = ask_runtime.block_on(core.ai_ask(question, "assistant-dialog".to_string()));
        app.set_assistant_busy(false);
        match result {
            Ok(answer) => app.set_assistant_answer(answer.into()),
            Err(error) => app.set_assistant_answer(error.into()),
        }
    });

    let app_weak = app.as_weak();
    let sum_state = Rc::clone(state);
    let sum_runtime = Rc::clone(runtime);
    app.on_assistant_summarise(move || {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let Some(core) = sum_state.borrow().core.clone() else {
            return;
        };
        let Some(thread_id) = selected_thread(&sum_state) else {
            app.set_assistant_answer(
                "Open a conversation first, then ask for a summary of it."
                    .to_string()
                    .into(),
            );
            return;
        };
        app.set_assistant_busy(true);
        let result = sum_runtime.block_on(core.ai_summarize(thread_id));
        app.set_assistant_busy(false);
        match result {
            Ok(summary) => app.set_assistant_answer(summary.into()),
            Err(error) => app.set_assistant_answer(error.into()),
        }
    });

    let app_weak = app.as_weak();
    let draft_state = Rc::clone(state);
    let draft_runtime = Rc::clone(runtime);
    app.on_assistant_draft(move |instruction| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let Some(core) = draft_state.borrow().core.clone() else {
            return;
        };
        let Some(thread_id) = selected_thread(&draft_state) else {
            app.set_assistant_answer(
                "Open the conversation you want replied to first."
                    .to_string()
                    .into(),
            );
            return;
        };
        app.set_assistant_busy(true);
        let result = draft_runtime.block_on(core.ai_draft(thread_id, instruction.trim()));
        app.set_assistant_busy(false);
        match result {
            // The draft lands in the answer box, not in an outbox. Copying it into a reply and
            // sending it stays a person's doing, whatever the agent level says.
            Ok(draft) => app.set_assistant_answer(draft.into()),
            Err(error) => app.set_assistant_answer(error.into()),
        }
    });

    let app_weak = app.as_weak();
    let plan_state = Rc::clone(state);
    let plan_runtime = Rc::clone(runtime);
    app.on_assistant_plan(move |prompt| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let Some(core) = plan_state.borrow().core.clone() else {
            return;
        };
        let prompt = prompt.trim().to_string();
        if prompt.is_empty() {
            return;
        }
        app.set_assistant_busy(true);
        let result = plan_runtime.block_on(core.ai_plan_automation(prompt));
        app.set_assistant_busy(false);
        match result {
            Ok(plan) => {
                app.set_assistant_plan_name(plan.name.clone().into());
                app.set_assistant_plan_supported(plan.supported);
                // An unsupported plan is not an error to hide: the reasons are what tell the
                // person how to say it differently.
                let detail = if plan.issues.is_empty() {
                    plan.summary.clone()
                } else {
                    plan.issues.join(" ")
                };
                app.set_assistant_plan_summary(detail.into());
                app.set_assistant_answer(plan.summary.into());
            }
            Err(error) => {
                app.set_assistant_plan_name(slint::SharedString::from(""));
                app.set_assistant_plan_supported(false);
                app.set_assistant_answer(error.into());
            }
        }
    });

    let app_weak = app.as_weak();
    let save_state = Rc::clone(state);
    let save_runtime = Rc::clone(runtime);
    app.on_assistant_save_plan(move || {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let Some(core) = save_state.borrow().core.clone() else {
            return;
        };
        let prompt = app.get_assistant_request().trim().to_string();
        if prompt.is_empty() {
            return;
        }
        app.set_assistant_busy(true);
        let result = save_runtime.block_on(core.save_automation_rule(prompt));
        app.set_assistant_busy(false);
        match result {
            Ok(name) => {
                app.set_assistant_plan_name(slint::SharedString::from(""));
                app.set_sync_status(UiMessage::detail("Rule saved: {}", name));
            }
            Err(error) => app.set_sync_status(UiMessage::detail("Could not save the rule: {}", error)),
        }
    });
}
