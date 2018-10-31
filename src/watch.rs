use futures::{Async, Future, Poll};

use actor::{Actor, AsyncContext};
use address::{channel, Addr, Recipient, RecipientRequest};
use arbiter::Arbiter;
use context::Context;
use contextimpl::ContextFut;
use handler::Message;
use mailbox::DEFAULT_CAPACITY;
use msgs::Execute;

#[derive(Debug,Clone,PartialEq)]
pub enum WatchEvent {
    Stopped,
}

impl Message for WatchEvent {
    type Result = ();
}

pub struct Watcher<A>
where
    A: Actor<Context = Context<A>>,
{
    fut: Option<ContextFut<A, Context<A>>>,
    subscribers: Vec<Recipient<WatchEvent>>,
    notifications: Vec<RecipientRequest<WatchEvent>>,
}

impl <A> Watcher<A>
where
    A: Actor<Context = Context<A>>,
{
    pub fn new(r: Recipient<WatchEvent>, f: ContextFut<A, Context<A>>) -> Watcher<A>
    where
        A: Actor<Context = Context<A>>,
    {
        Watcher {
            fut: Some(f),
            subscribers: vec![r],
            notifications: vec![],
        }
    }

    pub fn start<F>(r: Recipient<WatchEvent>, f: F) -> Addr<A>
    where
        F: FnOnce(&mut A::Context) -> A + 'static,
        A: Actor<Context = Context<A>>,
    {
        // create actor
        let mut ctx = Context::new();
        let act = f(&mut ctx);
        let addr = ctx.address();
        let fut = ctx.into_future(act);

        // create watcher
        Arbiter::spawn(Watcher::new(r, fut));

        addr
    }

    /// Start new supervised actor in arbiter's thread.
    pub fn start_in_arbiter<F>(sys: &Addr<Arbiter>, r: Recipient<WatchEvent>, f: F) -> Addr<A>
    where
        A: Actor<Context = Context<A>>,
        F: FnOnce(&mut Context<A>) -> A + Send + 'static,
    {
        let (tx, rx) = channel::channel(DEFAULT_CAPACITY);

        sys.do_send(Execute::new(move || -> Result<(), ()> {
            let mut ctx = Context::with_receiver(rx);
            let act = f(&mut ctx);
            let fut = ctx.into_future(act);

            Arbiter::spawn(Watcher::new(r, fut));
            Ok(())
        }));

        Addr::new(tx)
    }

    fn notify(&mut self, event: WatchEvent) {
        for s in self.subscribers.iter() {
            self.notifications.push(s.send(event.clone()));
        }
    }

    fn poll_for_event(&mut self) -> Option<WatchEvent> {
        if let Some(ref mut fut) = self.fut {
            match fut.poll() {
                Ok(Async::NotReady) => None,
                Ok(Async::Ready(_)) | Err(_) => {
                    Some(WatchEvent::Stopped)
                }
            }
        } else {
            None
        }
    }

    fn poll_notifications(&mut self) -> Poll<(), ()> {
        let mut notifications = self.notifications.split_off(0);

        while let Some(mut n) = notifications.pop() {
            match n.poll() {
                Ok(Async::NotReady) => {
                    self.notifications.push(n);
                },
                Ok(Async::Ready(_)) | Err(_) => {
                }
            }
        }

        if self.notifications.is_empty() {
            Ok(Async::Ready(()))
        } else {
            Ok(Async::NotReady)
        }
    }
}

#[doc(hidden)]
impl<A> Future for Watcher<A>
where
    A: Actor<Context = Context<A>>,
{
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(e) = self.poll_for_event() {
            self.notify(e);

            self.fut = None;
        }

        if self.fut.is_none() {
            self.poll_notifications()
        } else {
            Ok(Async::NotReady)
        }
    }
}