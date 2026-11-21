// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The server as the commands reach it: `samu_client::Client`, except that
//! one run of `samu` fetches the whole account only once.
//!
//! Most commands read `GET /changes?after=0`, and a short id is resolved
//! from the same answer first, so a command like `samu comments 5d85387d`
//! asked twice for the same two megabytes. The server hands the requests of
//! an account out one after another; the second one cost as much as the
//! first. The answer is kept until this run writes something; a run is too
//! short for another device's change to matter.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use samu_client::{Answer, Error, Result};
use serde_json::Value;

/// The request for the whole account.
const WHOLE: &str = "/changes?after=0";

/// What the server answered to [`WHOLE`].
#[derive(Debug)]
struct Kept {
    body: Vec<u8>,
    content_type: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct Client {
    inner: samu_client::Client,
    /// Shared by the clones: they are the same session.
    whole: Rc<RefCell<Option<Kept>>>,
}

impl Client {
    pub(crate) fn new(url: &str, token: Option<String>) -> Result<Self> {
        Ok(Self {
            inner: samu_client::Client::new(url, token)?,
            whole: Rc::default(),
        })
    }

    pub(crate) fn with_timeout(mut self, timeout: Duration) -> Self {
        self.inner = self.inner.with_timeout(timeout);
        self
    }

    /// After anything but a `GET` the account may look different.
    fn forget_after(&self, method: &str) {
        if method != "GET" {
            self.whole.borrow_mut().take();
        }
    }

    /// As `samu_client::Client::call`; the whole account comes from the
    /// answer kept, when there is one.
    pub(crate) fn call(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Answer> {
        if method != "GET" || path != WHOLE {
            self.forget_after(method);
            return self.inner.call(method, path, body);
        }
        if let Some(kept) = self.whole.borrow().as_ref() {
            return Ok(Answer {
                status: 200,
                body: kept.body.clone(),
                content_type: kept.content_type.clone(),
            });
        }
        let answer = self.inner.call(method, path, body)?;
        *self.whole.borrow_mut() = Some(Kept {
            body: answer.body.clone(),
            content_type: answer.content_type.clone(),
        });
        Ok(answer)
    }

    pub(crate) fn call_raw(
        &self,
        path: &str,
        content_type: &str,
        headers: &[(&str, String)],
        body: &[u8],
    ) -> Result<Answer> {
        self.forget_after("POST");
        self.inner.call_raw(path, content_type, headers, body)
    }

    pub(crate) fn call_bytes(
        &self,
        method: &str,
        path: &str,
        content_type: &str,
        headers: &[(&str, String)],
        body: &[u8],
    ) -> Result<Answer> {
        self.forget_after(method);
        self.inner
            .call_bytes(method, path, content_type, headers, body)
    }

    pub(crate) fn events<E: From<Error>>(
        &self,
        path: &str,
        on_event: impl FnMut(&str, &str) -> std::result::Result<bool, E>,
    ) -> std::result::Result<(), E> {
        self.inner.events(path, on_event)
    }
}
