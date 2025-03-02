#!/usr/bin/env python3

import os
import mysql.connector
import sys

CURDIR = os.path.dirname(os.path.realpath(__file__))
sys.path.insert(0, os.path.join(CURDIR, "../../../helpers"))

log = None

try:
    mydb = mysql.connector.connect(
        host="127.0.0.1", user="root", passwd="", port="3307", connection_timeout=6
    )
    cursor = mydb.cursor()

    cursor.execute("drop user if exists u1;")
    cursor.execute("drop user if exists u2;")
    cursor.execute("drop user if exists u3;")
    cursor.execute("create user u1 identified by 'abc123';")
    cursor.execute("drop network policy if exists p1;")
    cursor.execute("drop network policy if exists p2;")
    cursor.execute("create network policy p1 allowed_ip_list=('127.0.0.0/24');")
    cursor.execute(
        "create network policy p2 allowed_ip_list=('127.0.0.0/24') blocked_ip_list=('127.0.0.1');"
    )
    cursor.execute(
        "create user u2 identified by 'abc123' with set network policy='p1';"
    )
    cursor.execute(
        "create user u3 identified by 'abc123' with set network policy='p2';"
    )
except mysql.connector.errors.OperationalError:
    print("root@127.0.0.1 is timeout")

try:
    mydb = mysql.connector.connect(
        host="127.0.0.1", user="u1", passwd="abc123", port="3307", connection_timeout=3
    )
except mysql.connector.errors.OperationalError:
    print("u1 is timeout")

try:
    mydb = mysql.connector.connect(
        host="127.0.0.1", user="u2", passwd="abc123", port="3307", connection_timeout=3
    )
except mysql.connector.errors.OperationalError:
    print("u2 is timeout")

try:
    mydb = mysql.connector.connect(
        host="127.0.0.1", user="u3", passwd="abc123", port="3307", connection_timeout=3
    )
except mysql.connector.errors.OperationalError:
    print("u3 is timeout")
except mysql.connector.errors.ProgrammingError:
    print("u3 is blocked by client ip")
