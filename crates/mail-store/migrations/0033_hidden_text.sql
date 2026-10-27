-- Text the reader does not show but an agent would have read (spec
-- §14.4): invisible code points and elements styled not to show, found
-- with the sanitizer and left out of `text_plain`. JSON (`HiddenText`),
-- NULL when the body has none worth telling.
ALTER TABLE bodies ADD COLUMN hidden_json TEXT;
