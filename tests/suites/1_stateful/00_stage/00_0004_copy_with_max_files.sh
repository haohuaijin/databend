#!/usr/bin/env bash

CURDIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
. "$CURDIR"/../../../shell_env.sh


# Should be <root>/tests/data/

for force in 'false'  'true'
do
	for purge in 'false'  'true'
	do
		table="test_max_files_force_${force}_purge_${purge}"
		echo "drop table if exists ${table}" | $BENDSQL_CLIENT_CONNECT
		echo "CREATE TABLE ${table} (
      id INT,
      c1 INT
    ) ENGINE=FUSE;" | $BENDSQL_CLIENT_CONNECT
	done
done

rm -rf /tmp/00_0004
mkdir -p /tmp/00_0004

gen_files() {
cat << EOF > /tmp/00_0004/f1.csv
1,1
2,2
EOF

cat << EOF > /tmp/00_0004/f2.csv
3,3
4,4
EOF

cat << EOF > /tmp/00_0004/f3.csv
5,5
6,6
EOF
}

for force in 'false'  'true'
do
	for purge in 'false'  'true'
	do
		gen_files
		echo "--- force = ${force}, purge = ${purge}"
		for i in {1..3}
		do
			table="test_max_files_force_${force}_purge_${purge}"
			echo "copy into ${table} from 'fs:///tmp/00_0004/' FILE_FORMAT = (type = CSV) max_files=2 force=${force} purge=${purge}" | $BENDSQL_CLIENT_CONNECT
			echo "select count(*) from ${table}" | $BENDSQL_CLIENT_CONNECT
		  remain=$(ls -1 /tmp/00_0004/ | wc -l |  sed 's/ //g')
			echo "remain ${remain} files"
		done
	done
done

for force in 'false'  'true'
do
	for purge in 'false'  'true'
	do
		echo "drop table if exists test_max_files_force_${force}_purge_${purge}" | $BENDSQL_CLIENT_CONNECT
	done
done
