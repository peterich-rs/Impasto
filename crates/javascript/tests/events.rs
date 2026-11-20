#![cfg(feature = "runtime")]
#![allow(
    clippy::expect_used,
    reason = "assertions report native event regressions"
)]

use javascript::runtime::RuntimeBuilder;
use javascript::standard::StandardProfile;

#[test]
fn event_constructors_preserve_subclass_identity_and_native_brand() {
    let mut runtime = RuntimeBuilder::new()
        .standard(StandardProfile::portable())
        .build()
        .expect("the portable runtime starts");
    runtime
        .eval_script(
            r#"
        class Channel extends EventTarget {}
        class Message extends Event {}
        const channel = new Channel();
        const message = new Message('message');
        if (!(channel instanceof Channel && channel instanceof EventTarget &&
              message instanceof Message && message instanceof Event)) {
          throw Error('subclass lost its prototype');
        }
        for (const construct of [() => Event('missing new'), () => new AbortSignal(),
          () => EventTarget.prototype.dispatchEvent.call({}, message),
          () => channel.dispatchEvent({ type: 'message' })]) {
          let rejected = false;
          try { construct(); } catch (_) { rejected = true; }
          if (!rejected) throw Error('missing constructor or receiver validation');
        }
        let observed;
        const listener = { handleEvent(event) { observed = [this, event, event.currentTarget]; } };
        channel.addEventListener('message', listener);
        channel.dispatchEvent(message);
        if (observed[0] !== listener || observed[1] !== message || observed[2] !== channel ||
            message.currentTarget !== null || message.target !== channel) {
          throw Error('wrong object listener receiver');
        }
        "#,
            "event-subclass.js",
        )
        .expect("native event prototype and receiver behavior");
}

#[test]
fn event_options_and_abort_actions_preserve_callback_order() {
    let mut runtime = RuntimeBuilder::new()
        .standard(StandardProfile::portable())
        .build()
        .expect("the portable runtime starts");
    runtime
        .eval_script(
            r#"
        const target = new EventTarget();
        const controller = new AbortController();
        const calls = [];
        const listener = event => { event.preventDefault(); calls.push(event.type); };
        const duplicate = { get once() { throw Error('duplicate read once'); } };
        target.addEventListener('passive', listener, { passive: true });
        target.addEventListener('passive', listener, duplicate);
        if (!target.dispatchEvent(new Event('passive', { cancelable: true })))
          throw Error('passive listener canceled the event');
        target.addEventListener('aborted', listener, { signal: controller.signal });
        controller.signal.onabort = event => {
          event.stopImmediatePropagation();
          target.dispatchEvent(new Event('aborted'));
        };
        controller.abort();
        target.addEventListener('aborted', listener, {
          signal: controller.signal,
          get once() { throw Error('aborted read once'); }
        });
        const stopped = new Event('passive');
        stopped.cancelBubble = true;
        target.dispatchEvent(stopped);
        if (calls.join(',') !== 'passive' || stopped.cancelBubble)
          throw Error('event or abort lifecycle: ' + calls.join(','));
        "#,
            "event-options.js",
        )
        .expect("passive, duplicate, propagation and abort ordering");
}
