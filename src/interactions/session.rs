use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use secrecy::SecretString;

use crate::api::ApiClient;
use crate::error::{Error, Result};
use crate::ids::InteractionId;
use crate::models::{InteractionResponseType, Modal};

/// Secret credentials and response budget shared by all clones of an interaction.
#[derive(Debug, Clone)]
pub(crate) struct InteractionSession {
    id: InteractionId,
    token: SecretString,
    response_count: Arc<AtomicU8>,
}

impl InteractionSession {
    /// Create a session for one interaction event.
    pub(crate) fn new(id: InteractionId, token: SecretString) -> Self {
        Self {
            id,
            token,
            response_count: Arc::new(AtomicU8::new(0)),
        }
    }

    /// Shared response count, used by the runtime's automatic acknowledgement.
    pub(crate) fn response_count(&self) -> Arc<AtomicU8> {
        self.response_count.clone()
    }

    /// Send one callback response, claiming a response slot before I/O.
    ///
    /// Requests are not retried: after an ambiguous transport failure the
    /// server may already have accepted the callback.
    pub(crate) async fn respond(
        &self,
        api: &ApiClient,
        response_type: InteractionResponseType,
        content: Option<&str>,
    ) -> Result<()> {
        claim_interaction_response(&self.response_count)?;

        api.respond_interaction(&self.id, &self.token, response_type, content, None)
            .await?;

        Ok(())
    }

    /// Open a modal as the first response to this interaction.
    pub(crate) async fn respond_with_modal(&self, api: &ApiClient, modal: &Modal) -> Result<()> {
        claim_first_interaction_response(&self.response_count)?;
        let modal = serde_json::to_value(modal)?;
        api.respond_interaction(
            &self.id,
            &self.token,
            InteractionResponseType::Modal,
            None,
            Some(&modal),
        )
        .await?;
        Ok(())
    }
}

fn claim_first_interaction_response(response_count: &AtomicU8) -> Result<()> {
    response_count
        .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
        .map(|_| ())
        .map_err(|_| Error::InteractionAlreadyAnswered)
}

fn claim_interaction_response(response_count: &AtomicU8) -> Result<()> {
    let mut count = response_count.load(Ordering::Acquire);

    loop {
        if count >= 10 {
            return Err(Error::InteractionAlreadyAnswered);
        }

        match response_count.compare_exchange_weak(
            count,
            count + 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return Ok(()),
            Err(actual) => count = actual,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;
    use std::thread;

    use super::*;

    #[test]
    fn modal_can_only_claim_the_first_interaction_response() {
        let response_count = AtomicU8::new(0);
        assert!(claim_first_interaction_response(&response_count).is_ok());
        assert!(matches!(
            claim_first_interaction_response(&response_count),
            Err(Error::InteractionAlreadyAnswered)
        ));
        assert_eq!(response_count.load(Ordering::Acquire), 1);
    }

    #[test]
    fn interaction_allows_exactly_ten_response_claims() {
        let response_count = AtomicU8::new(0);

        for _ in 0..10 {
            assert!(claim_interaction_response(&response_count).is_ok());
        }

        assert!(matches!(
            claim_interaction_response(&response_count),
            Err(Error::InteractionAlreadyAnswered)
        ));
        assert_eq!(response_count.load(Ordering::Acquire), 10);
    }

    #[test]
    fn concurrent_interaction_response_claims_never_exceed_ten() {
        let response_count = Arc::new(AtomicU8::new(0));
        let successful_claims = Arc::new(AtomicUsize::new(0));
        let mut workers = Vec::new();

        for _ in 0..32 {
            let response_count = response_count.clone();
            let successful_claims = successful_claims.clone();

            workers.push(thread::spawn(move || {
                if claim_interaction_response(&response_count).is_ok() {
                    successful_claims.fetch_add(1, Ordering::Relaxed);
                }
            }));
        }

        for worker in workers {
            worker.join().unwrap();
        }

        assert_eq!(successful_claims.load(Ordering::Relaxed), 10);
        assert_eq!(response_count.load(Ordering::Acquire), 10);
    }
}
