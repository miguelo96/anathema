#!/bin/sh
echo "anathema: shell running as pid $$"
echo "anathema: uname: $(/usr/bin/uname -a)"
echo "anathema: root listing:"
/bin/ls -la /
echo "anathema: processes:"
/bin/ps -ax
echo "anathema: date: $(/bin/date)"
echo "anathema: done"
