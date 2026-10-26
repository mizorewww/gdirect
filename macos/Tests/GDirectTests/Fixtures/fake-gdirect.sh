#!/bin/sh

mode=${FAKE_GDIRECT_MODE:-success}

case "$mode" in
    cancel)
        trap '/bin/echo terminated > "$FAKE_TERM_FILE"; exit 42' TERM
        ;;
    ignore-term)
        trap '' TERM
        ;;
    race)
        trap 'exit 42' TERM
        ;;
esac

if [ -n "$FAKE_PID_FILE" ]; then
    /bin/echo $$ > "$FAKE_PID_FILE"
fi

case "$mode" in
    success)
        /bin/echo "stdout text"
        /bin/echo "stderr text" >&2
        ;;
    large)
        /usr/bin/awk 'BEGIN { for (i = 0; i < 16384; i++) printf "oooooooooooooooo" }'
        /usr/bin/awk 'BEGIN { for (i = 0; i < 16384; i++) printf "eeeeeeeeeeeeeeee" }' >&2
        ;;
    exit)
        /bin/echo "partial output"
        /bin/echo "failure detail" >&2
        exit 23
        ;;
    large-exit)
        /usr/bin/awk 'BEGIN { for (i = 0; i < 16384; i++) printf "eeeeeeeeeeeeeeee" }' >&2
        exit 2
        ;;
    query-only)
        /bin/echo -n 'https://example.com/upload?session='
        /usr/bin/awk 'BEGIN { for (i = 0; i < 50; i++) printf "ssssssssss" }'
        ;;
    list)
        if [ "$1" = "ls" ]; then
            /bin/echo '[{"id":"abc123","name":"资料.pdf","mimeType":"application/pdf","size":"42","modifiedTime":null,"md5Checksum":null,"resourceKey":null}]'
        else
            /bin/echo "not JSON and not a listing"
        fi
        ;;
    accounts)
        /bin/echo '* default@example.com'
        /bin/echo '  other@example.com'
        ;;
    invalid-list)
        /bin/echo '{"id":'
        ;;
    list-fail)
        /bin/echo '[{"id":"must-not-be-used","name":"bad","mimeType":"application/pdf"}]'
        /bin/echo "listing failed" >&2
        exit 9
        ;;
    stream)
        /bin/echo "ready"
        while [ ! -f "$FAKE_MARKER" ]; do
            /bin/sleep 0.02
        done
        /bin/echo "finished"
        /bin/echo "stream error" >&2
        ;;
    cancel)
        /bin/echo "ready"
        while :; do
            /bin/sleep 0.05
        done
        ;;
    ignore-term)
        /bin/echo "ready"
        while :; do
            /bin/sleep 0.2
        done
        ;;
    race)
        /bin/echo "ready"
        /bin/sleep 0.01
        ;;
    logs)
        /usr/bin/awk 'BEGIN { for (i = 0; i < 400; i++) printf "aaaaaaaaaa" }'
        /bin/echo ' https://accounts.google.com/o/oauth2/auth?client_id=secret-client&state=secret-state'
        /bin/echo "stdout-end"
        /usr/bin/awk 'BEGIN { for (i = 0; i < 400; i++) printf "bbbbbbbbbb" }' >&2
        /bin/echo ' https://example.com/upload?session=secret-session' >&2
        /bin/echo "stderr-end" >&2
        ;;
    *)
        /bin/echo "unknown fake mode" >&2
        exit 64
        ;;
esac
