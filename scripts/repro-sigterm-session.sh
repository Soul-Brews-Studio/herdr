#!/usr/bin/env zsh
# Scratch reproduction of the white laris-co reboot (2026-10-29): SIGTERM the server AND every process
# under it at once (what systemd does to a unit at shutdown), then start the server again.
#   zsh scripts/repro-sigterm-session.sh <herdr binary> <scratch session name>
# Prints: spaces + agent ids before, whether session.json survived, the persist log lines, and the
# spaces + agent ids after the restart. Scratch only: it deletes the session at the end.
set -u
H=${1:?herdr binary}; S=${2:?scratch session}; T=/private/tmp/$S
hs() { $H --session $S "$@"; }
spaces() { hs workspace list | python3 -c "import json,sys; print(' '.join(w['label'] for w in json.load(sys.stdin)['result']['workspaces']))"; }
agents() { hs agent list | python3 -c "
import json,sys
print(' '.join(a['pane_id']+'='+((a.get('agent_session') or {}).get('value') or '?')[:8] for a in json.load(sys.stdin)['result']['agents']))"; }
start() { (env -u HERDR_SOCKET_PATH -u HERDR_PANE_ID -u HERDR_SESSION -u HERDR_ENV nohup $H --session $S server >/dev/null 2>&1 &)
  for i in $(seq 1 40); do hs pane list >/dev/null 2>&1 && return 0; sleep 0.5; done; return 1; }

$H session stop $S >/dev/null 2>&1; $H session delete $S >/dev/null 2>&1; rm -rf $T; mkdir -p $T/a $T/b
(cd $T/a && git init -q && git commit -q --allow-empty -m i)
start || { echo "server did not start"; exit 1; }
DIR=$($H session list | awk -v s=$S '$1==s{print $3}')   # a debug build keeps sessions in ~/.config/herdr-dev
hs workspace create --cwd $T/a --label space-a --no-focus >/dev/null
hs workspace create --cwd $T/b --label space-b --no-focus >/dev/null
P=$(hs pane list | python3 -c "import json,sys; print([p['pane_id'] for p in json.load(sys.stdin)['result']['panes'] if p['cwd'].endswith('/a')][0])")
hs agent start sig-agent --kind claude --pane $P --timeout 60000 -- --model haiku >/dev/null 2>&1
sleep 3; hs pane send-keys $P down >/dev/null 2>&1; hs pane send-keys $P enter >/dev/null 2>&1
hs pane wait-output $P --match 'for shortcuts' --timeout 40000 >/dev/null 2>&1
hs agent prompt $P "Reply with just: ok" --wait >/dev/null 2>&1
sleep 6   # let the debounced session save land
echo "before:  spaces: $(spaces)"
echo "before:  agents: $(agents)"

SPID=$(pgrep -f -- "--session $S server" | head -1)
ALL=$(python3 - "$SPID" <<'PY'
import subprocess,sys
root=int(sys.argv[1]); rows=[l.split() for l in subprocess.run(["ps","-axo","pid=,ppid="],capture_output=True,text=True).stdout.splitlines()]
kids={}; [kids.setdefault(int(pp),[]).append(int(p)) for p,pp in rows]
out=[root]; i=0
while i<len(out): out+=kids.get(out[i],[]); i+=1
print(" ".join(map(str,out)))
PY
)
KIDS=${ALL#$SPID }
# the white order: every pane's process dies first (HUP'd shells, killed agents), the server gets its
# SIGTERM a moment later — inside the 5 s save debounce
echo "kill -HUP: $(echo $KIDS | wc -w | tr -d ' ') pane processes, then kill -TERM server $SPID 0.5 s later"
kill -HUP ${=KIDS} 2>/dev/null; sleep 0.5; kill -TERM $SPID 2>/dev/null
for i in $(seq 1 40); do kill -0 $SPID 2>/dev/null || break; sleep 0.25; done
[ -f $DIR/session.json ] && echo "after TERM: session.json kept" || echo "after TERM: session.json MISSING"
[ -f $DIR/session-cleared ] && echo "after TERM: session-cleared marker present" || echo "after TERM: no session-cleared marker"
rg -o -N '"(session cleared|no spaces left during shutdown[^"]*|session saved)"' $DIR/herdr-server.log 2>/dev/null | tail -3 | sed 's/^/  log: /'

start || { echo "restart failed"; exit 1; }
sleep 12   # resume_agents_on_restore: give the claude a moment to come back
echo "after restart: spaces: $(spaces)"
echo "after restart: agents: $(agents)"
rg -o -N '"session file is missing[^"]*"' $DIR/herdr-server.log 2>/dev/null | tail -1 | sed 's/^/  log: /'

cp $DIR/herdr-server.log /tmp/$S-herdr-server.log 2>/dev/null && echo "  (server log kept: /tmp/$S-herdr-server.log)"
$H session stop $S >/dev/null 2>&1; $H session delete $S >/dev/null 2>&1; rm -rf $T
