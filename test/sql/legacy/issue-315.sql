CREATE TABLE a (
  pk_a        BIGINT NOT NULL,
  fk_a_to_map BIGINT
);

CREATE TABLE mapping (
  pk_map bigint NOT NULL,
  fk_map_to_b bigint
);

CREATE TABLE b (
  pk_b_id    BIGINT NOT NULL,
  b_group_id varchar
);

create index idxb on b using zombodb ( (b.*) );
create index idxmapping on mapping using zombodb ( (mapping.*) );
create index idxa on a using zombodb ( (a.*) ) with (
      options='fk_a_to_map=<public.mapping.idxmapping>pk_map,
               fk_map_to_b=<public.b.idxb>pk_b_id');


insert into b(pk_b_id, b_group_id) values(100, 42);
insert into b(pk_b_id, b_group_id) values(200, 42);
insert into b(pk_b_id, b_group_id) values(300, 0);

insert into mapping(pk_map, fk_map_to_b) values(1, 100);
insert into mapping(pk_map, fk_map_to_b) values(2, 300);
insert into mapping(pk_map, fk_map_to_b) values(3, 200);

insert into a (pk_a, fk_a_to_map) values(10, 1);
insert into a (pk_a, fk_a_to_map) values(20, 2);
insert into a (pk_a, fk_a_to_map) values(30, 3);

select * from a where a ==> '#expand<b_group_id=<this.index>b_group_id>(pk_b_id = 100)' order by pk_a;

set zdb.ignore_visibility to on; /* to avoid transient xact data in query output */
select json_array_element((zdb.dump_query('idxa', '#expand<b_group_id=<this.index>b_group_id>(pk_b_id = 100)')::json)->'subselect'->'query'->'bool'->'should', 0)->'subselect'->'query';

drop table a cascade;
drop table b cascade;
drop table mapping cascade;