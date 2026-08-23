-- Wrapping a multi-statement migration in a transaction makes the whole
-- migration body succeed or fail as a unit.
BEGIN;

DEFINE TABLE users SCHEMAFULL;
DEFINE FIELD name ON users TYPE string;
DEFINE FIELD email ON users TYPE string;
DEFINE INDEX email_idx ON users FIELDS email UNIQUE;

COMMIT;
