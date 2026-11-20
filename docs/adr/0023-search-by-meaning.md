# ADR 0023: Search by meaning with Apple's on-device sentence model

- Status: Accepted
- Date: 2026-11-18
- Spec: §14.3 amendment 2026-11-18

## Context

Search finds words. The user often remembers what an email was about,
not its words ("the thing about the office move"). Kaluta has no server,
and mail must not leave the Mac to be indexed.

## Decision

1. Each message's subject and snippet becomes a vector from Apple's
   on-device English sentence model (`NLEmbedding.sentenceEmbedding`),
   computed by the app in the background, newest mail first, in batches
   with pauses, while Kaluta runs. The core keeps them in
   `message_embeddings` (migration 0044) by model name, so a new model
   starts over. They are derived data: no undo record, rebuilt freely.
2. A search with plain words (no operators) shows its word matches first;
   once all of them are shown, the conversations nearest the query's
   meaning that are not among them follow, marked *Related*. The core
   ranks by cosine over every stored vector (brute force, in Rust) and
   keeps each conversation's nearest message; Spam and Trash are left out
   as search leaves them out.
3. The plan named `NLContextualEmbedding`. The sentence model is used
   instead: it gives one vector per text with no pooling, is on every
   supported Mac without a download, and is enough for subject and
   snippet. A better model can replace it under a new model name.

## Consequences

- Nothing about the mail leaves the Mac.
- Only English is understood well; other languages fall back to words.
- Only subject and snippet are read, not whole bodies or attachments.
- The agent's `mail_search` does not search by meaning yet: the core
  cannot compute a query's vector, only the app can.
- A Mac without the English model indexes nothing and search is as
  before.
