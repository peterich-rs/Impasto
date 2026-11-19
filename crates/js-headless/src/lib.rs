//! A consumer that can see only `js`. It has no session, tree, or reactive graph.

pub fn started() -> Result<(), js::JsError> {
    let mut runtime = js::RuntimeBuilder::new().build()?;
    runtime.eval_script("globalThis.value = 1 + 2;", "headless.js")?;
    runtime.drive(0)?;
    runtime.shutdown();
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn consumer_runs_without_a_ui_crate() {
        super::started().expect("headless");
    }
}
