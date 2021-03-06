CREATE TABLE a (
  id SERIAL8 PRIMARY KEY
);
CREATE TABLE b (
  id SERIAL8 PRIMARY KEY
);
CREATE TABLE c (
  id SERIAL8 PRIMARY KEY
);

CREATE INDEX idxc ON c USING zombodb((c.*));

CREATE FUNCTION c_shadow (anyelement) RETURNS anyelement IMMUTABLE STRICT PARALLEL SAFE LANGUAGE c AS '$libdir/zombodb.so', 'shadow_wrapper';
CREATE INDEX idxc_shadow ON c USING zombodb (c_shadow(c.*)) with (shadow=true);

CREATE VIEW issue_58_view AS
  SELECT
    a.id                       AS a_id,
    b.id                       AS b_id,
    c.id                       AS c_id,
    c_shadow(c)                AS zdb
  FROM a, b, c;

set enable_seqscan to off;
set ENABLE_BITMAPSCAN to off;
explain (costs off) select * from issue_58_view where zdb==>'';
select assert(zdb.determine_index('issue_58_view')::regclass, 'idxc_shadow'::regclass, 'Found correct index');

DROP VIEW issue_58_view;
DROP TABLE a;
DROP TABLE b;
DROP TABLE c CASCADE;
DROP FUNCTION c_shadow;