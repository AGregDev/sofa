#!/usr/bin/env bash
# Sofa's automated QA run on Linux: the same run as qa\qa.ps1, with the same check names. It
# starts Sofa in QA mode (--qa: its own folder in $TMPDIR/sofa-qa, nothing launched, nothing
# outside that folder touched) in a 1600x900 window, drives every screen through the QA control
# pipe ($TMPDIR/sofa-qa/control), checks what Sofa reports in its log, saves screenshots to
# shots/linux/, and measures start-up time, memory and idle CPU. Writes shots/linux/qa.txt.
# Exit code 0 = all pass.
#
#   qa/qa.sh [path/to/sofa] [out-dir]
#
# Needs an X server (CI runs it under xvfb-run), jq, and xmessage and xwininfo (x11-utils) for
# the stand-in window of the Running checks.
set -u
EXE=$(realpath "${1:-${CARGO_TARGET_DIR:-target}/release/sofa}")
OUT=${2:-shots/linux}
QA="${TMPDIR:-/tmp}/sofa-qa"
LOG="$QA/qa.log"
JSON="$QA/sofa.json"
mkdir -p "$OUT"
OUT=$(realpath "$OUT")
rm -f "$OUT"/*.png
RESULTS=()
FAILS=0

check() { # name pass(1/0) [detail]
  local mark=PASS
  [ "$2" = 1 ] || { mark=FAIL; FAILS=$((FAILS + 1)); }
  local line="$mark  $1"
  [ -n "${3:-}" ] && line="$line  ($3)"
  RESULTS+=("$line")
  echo "$line"
}
# c name 'condition' [detail]: the condition is evaluated as written (so && and || stay in it).
c() { if eval "$2"; then check "$1" 1 "${3:-}"; else check "$1" 0 "${3:-}"; fi; }

# What a QA run must never touch: the real settings and key, start with the session, the
# desktop entry, the installed copy, and Steam's PS-button setting.
fingerprint() {
  local f out=""
  for f in "$HOME/.config/sofa/sofa.json" "$HOME/.config/sofa/key" "$HOME/.config/autostart/sofa.desktop" "$HOME/.local/share/applications/sofa.desktop" "$HOME/.local/bin/sofa"; do
    if [ -e "$f" ]; then out="$out|$f=$(sha256sum "$f" | cut -c1-16)"; else out="$out|$f=none"; fi
  done
  for f in "$HOME"/.local/share/Steam/userdata/*/config/localconfig.vdf "$HOME"/.var/app/com.valvesoftware.Steam/.local/share/Steam/userdata/*/config/localconfig.vdf; do
    [ -e "$f" ] && out="$out|steam=$(grep -o '"Controller_CheckGuideButton"[[:space:]]*"[0-9]*"' "$f" | head -1)"
  done
  echo "$out"
}

ctl() { printf '%s\n' "$*" > "$QA/control"; }
pause() { sleep "$(awk "BEGIN{print $1/1000}")"; }
key() { # name [times] [wait ms]
  local n=${2:-1} w=${3:-260} i
  for ((i = 0; i < n; i++)); do ctl "key $1"; pause "$w"; done
}
type_text() { ctl "type $1"; pause 400; }
# The last state line, as key=value pairs in the associative array S.
declare -A S
state() {
  S=()
  local line kv parts
  line=$(grep 'state|' "$LOG" 2> /dev/null | tail -1)
  line=${line#*state|}
  IFS='|' read -ra parts <<< "$line"
  for kv in "${parts[@]}"; do S["${kv%%=*}"]="${kv#*=}"; done
}
setting() { jq -r "$1" "$JSON" 2> /dev/null; }
override() { jq -r --arg t "$1" ".overrides[\$t]$2" "$JSON" 2> /dev/null; }
shot() { # name [settle ms]
  pause "${2:-900}"
  ctl "shot $1"
  for _ in $(seq 1 30); do [ -f "$QA/shots/$1.png" ] && break; sleep 0.1; done
  cp "$QA/shots/$1.png" "$OUT/$1.png" 2> /dev/null || true
}
rss_mb() { awk '/^VmRSS/ {printf "%.1f", $2 / 1024}' "/proc/$1/status"; }
cpu_ticks() { awk '{print $14 + $15}' "/proc/$1/stat"; }
threads() { awk '/^Threads/ {print $2}' "/proc/$1/status"; }
gone() { # pid [seconds]: waits for the process to end
  local i
  for ((i = 0; i < ${2:-4} * 4; i++)); do kill -0 "$1" 2> /dev/null || return 0; sleep 0.25; done
  return 1
}

start_sofa() {
  rm -f "$LOG"
  "$EXE" --qa --size 1600x900 >> "$OUT/stderr.txt" 2>&1 &
  PID=$!
  for _ in $(seq 1 60); do
    grep -q 'ready in' "$LOG" 2> /dev/null && [ -p "$QA/control" ] && break
    sleep 0.25
  done
}

[ -x "$EXE" ] || { echo "FAIL  build first: $EXE not found"; exit 1; }
BEFORE=$(fingerprint)
rm -rf "$QA"
: > "$OUT/stderr.txt"

# A few made-up desktop entries, so the library has games, a player and an app to sort and
# search even on a bare system (a CI container), and an SVG icon to draw. They live in the QA
# folder and reach Sofa only through this run's XDG_DATA_DIRS.
FIX="$QA/xdg"
mkdir -p "$FIX/applications" "$FIX/icons/hicolor/scalable/apps"
printf '%s\n' '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect x="4" y="4" width="56" height="56" rx="14" fill="#f0a24b"/><circle cx="32" cy="32" r="13" fill="#1c1a18"/></svg>' > "$FIX/icons/hicolor/scalable/apps/qa-sofa-test.svg"
entry() { printf '[Desktop Entry]\nType=Application\nName=%s\nExec=true\nIcon=qa-sofa-test\nCategories=%s\n' "$2" "$3" > "$FIX/applications/$1.desktop"; }
entry qa-racer 'Arcade Racer' 'Game;ArcadeGame;'
entry qa-tower 'Puzzle Tower' 'Game;LogicGame;'
entry qa-kart 'Kart Party' 'Game;'
entry qa-player 'Video Player' 'AudioVideo;Player;'
entry qa-notes 'Notes' 'Utility;'
export XDG_DATA_DIRS="$FIX:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"

# ---------- First run ----------
start_sofa
c 'S1 starts and opens its window' 'kill -0 "$PID" && grep -q "ready in" "$LOG"'
MS=$(grep -o 'ready in [0-9]* ms' "$LOG" 2> /dev/null | head -1 | grep -o '[0-9][0-9]*')
c 'S2 starts in under 1.5 s' '[ "${MS:-99999}" -lt 1500 ]' "${MS:-?} ms to the first frame"
sleep 1.2
key Home
state
c 'W1 first run shows the welcome screen' '[ "${S[layer]:-}" = "welcome:3" ]' "${S[layer]:-}"
shot 01-welcome 600
key Up
key Enter
c 'W2 welcome: button layout cycles' '[ "$(setting .settings.controllerLayout)" = ps ]' "$(setting .settings.controllerLayout)"
key Down
key Enter
state
c 'W3 welcome: Continue opens Home' '[ "${S[layer]:-}" = none ] && [ "${S[toast]:-}" = "Sofa is ready" ]' "${S[layer]:-}, toast '${S[toast]:-}'"
c 'W4 welcome: setup is saved' '[ "$(setting .setupDone)" = true ]'

# ---------- Home ----------
sleep 1.5
MENU="$QA/cache/startmenu.json"
for _ in $(seq 1 40); do [ -f "$MENU" ] && break; sleep 0.25; done
c 'L1 The desktop entries are listed in the background' '[ -f "$MENU" ] && jq -e "map(select(.name == \"Arcade Racer\" and .category == \"games\" and (.icon // \"\" | endswith(\".png\")))) | length == 1" "$MENU" > /dev/null' "$([ -f "$MENU" ] && jq length "$MENU") entries"
sleep 0.8
c 'L2 The scan helper exits when done' '! pgrep -f -- "--scan $QA" > /dev/null'
state
c 'H1 Home has rows and the library' '[ "${S[rows]:-0}" -ge 1 ] && [ "${S[items]:-0}" -ge 12 ]' "rows ${S[rows]:-}, items ${S[items]:-}"
shot 02-home 1500
key Right
state
c 'H2 Right moves along the row' '[ "${S[focus]:-}" = 0,1 ]' "${S[focus]:-}"
key Down
state
ROWS=${S[rows]:-0}
c 'H3 Down moves to the next row' '[ "$ROWS" -lt 2 ] || [[ "${S[focus]:-}" == 1,* ]]' "${S[focus]:-}"
shot 03-home-row2
key Esc
state
c 'H4 Back on Home returns to the first tile' '[ "${S[focus]:-}" = 0,0 ] && [ "${S[tab]:-}" = home ] && [ "${S[layer]:-}" = none ]' "${S[tab]:-} ${S[focus]:-} ${S[layer]:-}"

# ---------- Quick menu (PS on Home) ----------
# A QA run has a made-up track playing ("Night Drive"), made-up sound devices (a TV and
# headphones, 40 %, microphone on) and changes only those; the sleep timer never sleeps the PC.
key Home
state
c 'U1 PS on Home opens the quick menu, what is playing first' '[ "${S[layer]:-}" = quick:music ]' "${S[layer]:-}"
shot 16-quick-menu 700
key Enter
state
c 'U2 Cross pauses what is playing' '[ "${S[music]:-}" = paused ]' "music ${S[music]:-}"
key Enter
key Right 2
state
c 'U3 Right moves along the quick menu' '[ "${S[layer]:-}" = quick:volume ]' "${S[layer]:-}"
key VolUp
state
c 'U4 R2 raises the volume a step' '[ "${S[volume]:-}" = 44 ]' "volume ${S[volume]:-}"
key VolDown
key Right
key Enter
state
c 'U5 The microphone mutes' '[ "${S[mic]:-}" = muted ]' "mic ${S[mic]:-}"
key Enter
key Right
key Enter
state
c 'U6 The sleep timer starts at 30 minutes' '[ "${S[sleep]:-}" = 30 ] && [ "${S[layer]:-}" = quick:timer ]' "sleep ${S[sleep]:-}, ${S[layer]:-}"
shot 17-quick-timer 500
key Enter 3
state
c 'U7 The sleep timer steps up to 2 hours' '[ "${S[sleep]:-}" = 120 ]' "sleep ${S[sleep]:-}"
key Enter
state
c 'U8 then turns off' '[ "${S[sleep]:-}" = off ]' "sleep ${S[sleep]:-}"
key Left 3
key Enter
state
c 'U9 Sound output lists the outputs, the one in use in focus' '[ "${S[layer]:-}" = "sheet:Sound output:0" ]' "${S[layer]:-}"
shot 18-sound-output 500
key Down
key Enter
state
c 'U10 Choosing an output switches the sound and returns to the menu' '[ "${S[output]:-}" = qa-headphones ] && [ "${S[layer]:-}" = quick:output ]' "${S[output]:-}, ${S[layer]:-}"
key Home
state
c 'U11 PS again closes the quick menu' '[ "${S[layer]:-}" = none ]' "${S[layer]:-}"
ctl sleep
pause 500
state
c 'U12 The sleep timer warns a minute before the PC sleeps' '[[ "${S[layer]:-}" == "dialog:Sleeping in"* ]]' "${S[layer]:-}"
shot 19-sleep-warning 600
key Enter
state
c 'U13 Stay awake turns the timer off' '[ "${S[sleep]:-}" = off ] && [ "${S[layer]:-}" = none ]' "sleep ${S[sleep]:-}, ${S[layer]:-}"

# ---------- Performance (quick menu) ----------
# CPU, GPU, memory and video memory are read from this computer (nothing changes). The overlay is
# a window of its own (override-redirect); there is no frame rate on Linux.
key Home
key Right 6
key Enter
state
c 'U14 Performance shows CPU, GPU, memory and video memory' '[ "${S[layer]:-}" = "sheet:Performance:0" ]' "${S[layer]:-}"
shot 22-performance 900
key Enter
sleep 1.2
state
MAPPED=$(xwininfo -name "Sofa overlay" 2>/dev/null | grep -o 'IsViewable\|IsUnMapped\|IsUnviewable' || echo none)
c 'U15 The overlay shows in a window of its own (saved)' '[ "$(setting .settings.perfOverlay)" = true ] && [ "${S[overlay]:-}" = on ] && [ "$MAPPED" = IsViewable ]' "$(setting .settings.perfOverlay), overlay ${S[overlay]:-}, window $MAPPED"
key Enter
key Esc
key Home
state
MAPPED=$(xwininfo -name "Sofa overlay" 2>/dev/null | grep -o 'IsViewable\|IsUnMapped\|IsUnviewable' || echo none)
c 'U16 and off again' '[ "$(setting .settings.perfOverlay)" = false ] && [ "${S[overlay]:-}" = off ] && [ "$MAPPED" != IsViewable ] && [ "${S[layer]:-}" = none ]' "$(setting .settings.perfOverlay), overlay ${S[overlay]:-}, window $MAPPED, ${S[layer]:-}"

# ---------- Tabs ----------
TABS=""
for _ in $(seq 1 7); do
  key E 1 450
  state
  TABS="$TABS${TABS:+,}${S[tab]:-}"
  case "${S[tab]:-}" in games) shot 04-games ;; social) shot 05-social ;; system) shot 05-system ;; esac
done
c 'T1 R1 walks the tabs and wraps' '[ "$TABS" = games,media,music,apps,social,system,home ]' "$TABS"
key Q
state
c 'T2 L1 goes back a tab' '[ "${S[tab]:-}" = system ]' "${S[tab]:-}"
key Esc
state
c 'T3 Back returns to Home' '[ "${S[tab]:-}" = home ]' "${S[tab]:-}"

# ---------- Spotify (Music tab) ----------
# A QA run has a made-up library (Liked Songs, three playlists, two albums) and plays nothing.
key E 3
state
c 'SP1 The Music tab lists Liked Songs, playlists and albums' '[ "${S[tab]:-}" = music ] && [ "${S[rowkeys]:-}" = playlists,albums ] && [ "${S[tile]:-}" = mu:spotify:user:qa:collection ]' "${S[tab]:-} ${S[rowkeys]:-} ${S[tile]:-}"
shot 20-music 900
key Enter
state
c 'SP2 Cross plays it on this PC' '[ "${S[toast]:-}" = "Playing Liked Songs" ]' "${S[toast]:-}"
key Right
key M
state
c 'SP3 Triangle offers Play, Shuffle play and Open in Spotify' '[ "${S[layer]:-}" = "sheet:Late Night Drive:0" ]' "${S[layer]:-}"
shot 21-music-options 500
key Esc
key Esc
state
c 'SP4 Home has a Music row' '[[ ",${S[rowkeys]:-}," == *,music,* ]]' "${S[rowkeys]:-}"

# ---------- Settings ----------
key O
state
c 'C1 Options opens Settings' '[ "${S[layer]:-}" = "sheet:Settings:0" ]' "${S[layer]:-}"
shot 06-settings 500
key Down 8
state
c 'C2 Settings: focus moves down the list' '[ "${S[layer]:-}" = "sheet:Settings:8" ]' "${S[layer]:-}"
key Enter
c 'C3 Settings: Reduce motion switches on' '[ "$(setting .settings.motion)" = reduced ]' "$(setting .settings.motion)"
key Enter
c 'C4 Settings: and off again' '[ "$(setting .settings.motion)" = full ]' "$(setting .settings.motion)"
key Down
key Enter
c 'C5 Settings: Show hidden apps' '[ "$(setting .settings.showHidden)" = true ]'
key Enter
key Up 7
key Enter
c 'C6 Settings: Home button choice' '[ "$(setting .settings.homeButton)" = combo ]' "$(setting .settings.homeButton)"
key Enter
c 'C7 Settings: back to the PS button' '[ "$(setting .settings.homeButton)" = ps ]' "$(setting .settings.homeButton)"
key Down 2
key Enter
LIGHT=$(setting .settings.controllerLight)
key Enter
LIGHT="$LIGHT,$(setting .settings.controllerLight)"
key Enter
LIGHT="$LIGHT,$(setting .settings.controllerLight)"
c 'C9 Settings: Controller light cycles (saved)' '[ "$LIGHT" = amber,off,follow ]' "$LIGHT"
key Esc
state
c 'C8 Back closes Settings' '[ "${S[layer]:-}" = none ]' "${S[layer]:-}"

# ---------- Search ----------
key F
state
c 'K1 Square opens search' '[[ "${S[layer]:-}" == keyboard:Search:* ]]' "${S[layer]:-}"
shot 07-search-empty 400
type_text a
state
HITS=$(echo "${S[layer]:-}" | cut -d: -f4)
c 'K2 Typing finds games and apps' '[ "${HITS:-0}" -gt 0 ]' "${S[layer]:-}"
shot 08-search 700
key Back
state
c 'K3 Backspace deletes' '[[ "${S[layer]:-}" == keyboard:Search::* ]]' "${S[layer]:-}"
key Down
key Enter
state
c 'K4 On-screen keys type' '[[ "${S[layer]:-}" == keyboard:Search:z:* ]]' "${S[layer]:-}"
key Esc
state
c 'K5 Back closes the keyboard' '[ "${S[layer]:-}" = none ]' "${S[layer]:-}"

# ---------- Item options ----------
key Esc
state
TILE=${S[tile]:-}
if [[ "$TILE" == nav:* || "$TILE" == system:* || -z "$TILE" ]]; then
  key Right
  state
  TILE=${S[tile]:-}
fi
key M
state
c 'I1 Triangle opens the item menu' '[[ "${S[layer]:-}" == sheet:*:0 && "${S[layer]:-}" != "sheet:Settings:0" ]]' "${S[layer]:-}"
shot 09-item-menu 500
key Enter
c 'I2 Add to favorites' '[ "$(override "$TILE" .favorite)" = true ]' "$TILE"
shot 10-favorites 1200
key M
key Enter
c 'I3 Remove from favorites' '[ "$(override "$TILE" "")" = null ]' "$TILE"
key M
key Down 3
key Enter
state
c 'I4 Rename opens the keyboard with the name' '[[ "${S[layer]:-}" == keyboard:Rename* ]]' "${S[layer]:-}"
key Back 60 25
type_text 'QA Name'
shot 11-rename 300
key Enter
c 'I5 Rename saves the name' '[ "$(override "$TILE" .name)" = "QA Name" ]' "$(override "$TILE" .name)"
key M
key Down 3
key Enter
key Back 60 25
key Enter
pause 400
c 'I6 An empty name brings the original back' '[ "$(override "$TILE" "")" = null ]' "$TILE"

# ---------- Launch ----------
key Esc
state
TILE=${S[tile]:-}
key Enter 1 500
state
c 'G1 Select shows the launch screen (nothing starts in QA)' '[[ "${S[layer]:-}" == launch:* ]]' "${S[layer]:-}"
shot 12-launch 400
key Esc
state
c 'G2 Back leaves the launch screen' '[ "${S[layer]:-}" = none ]' "${S[layer]:-}"
c 'G3 Launching records Jump back in' '[ "$(jq -r --arg t "$TILE" ".recent[\$t]" "$JSON")" != null ]' "$TILE"

# ---------- Running card ----------
xmessage -name sofaqa -title 'Sofa QA window' 'Sofa QA window' > /dev/null 2>&1 &
STAND=$!
WIN=""
for _ in $(seq 1 40); do
  WIN=$(xwininfo -root -tree 2> /dev/null | awk '/"Sofa QA window"/ {print $1; exit}')
  [ -n "$WIN" ] && break
  sleep 0.25
done
WIN=$((${WIN:-0}))
ctl "run $WIN"
pause 500
state
c 'R1 PS from an app adds a Running card first' '[ "${S[running]:-}" = 1 ] && [ "${S[tile]:-}" = "run:$WIN" ] && [ "${S[focus]:-}" = 0,0 ]' "${S[tile]:-} running ${S[running]:-}"
shot 13-running 900
key M
state
c 'R2 Triangle on Running opens its menu' '[[ "${S[layer]:-}" == sheet:*:0 && "${S[layer]:-}" != "sheet:Settings:0" ]]' "${S[layer]:-}"
shot 14-running-menu 400
key Esc
# A game given the focus with the button still down reads a fresh press: the app comes forward
# only once the key is up.
ctl "down Enter"
pause 500
state
c 'R4 Select held on Running: the app waits for the release' '[ "${S[resume]:-}" = "wait:$WIN" ]' "${S[resume]:-}"
ctl "up Enter"
pause 400
state
c 'R5 Released, the app comes forward' '[ "${S[resume]:-}" = "done:$WIN" ]' "${S[resume]:-}"
kill "$STAND" 2> /dev/null
pause 400
key Home # PS on Home: the quick menu opens, and Running drops apps that closed
state
c 'R3 A closed app loses its card' '[ "${S[running]:-}" = 0 ]' "running ${S[running]:-}"
key Home

# ---------- One Sofa at a time ----------
"$EXE" --qa --size 1600x900 > /dev/null 2>&1 &
SECOND=$!
c 'O1 A second start hands over to the first' 'gone "$SECOND" 4 && kill -0 "$PID"'

# ---------- Memory and CPU ----------
key Esc
sleep 3
RSS=$(rss_mb "$PID")
c 'P1 Memory on Home under 150 MB' 'awk "BEGIN{exit !($RSS < 150)}"' "resident $RSS MB"
C0=$(cpu_ticks "$PID")
sleep 5
C1=$(cpu_ticks "$PID")
CPU=$(awk "BEGIN{printf \"%.2f\", ($C1 - $C0) / $(getconf CLK_TCK) / 5 * 100}")
c 'P2 Idle CPU under 1% of a core' 'awk "BEGIN{exit !($CPU < 1.0)}"' "$CPU %"
TH=$(threads "$PID")
c 'P3 Threads stay few' '[ "$TH" -le 24 ]' "$TH threads"
ctl trim
sleep 2
RSS2=$(rss_mb "$PID")
c 'P4 Memory is given back in the background' 'awk "BEGIN{exit !($RSS2 < $RSS)}"' "resident $RSS MB -> $RSS2 MB"
key Right
key Left
shot 15-after-trim 1500
state
c 'P5 Draws again after giving memory back' 'kill -0 "$PID" && [ "${S[focus]:-}" = 0,0 ]'

# ---------- Quit ----------
key O
key Down 12
state
c 'Q1 Quit Sofa is the last setting' '[ "${S[layer]:-}" = "sheet:Settings:12" ]' "${S[layer]:-}"
key Enter
c 'Q2 Quit Sofa closes it' 'gone "$PID" 4'

# ---------- Second start ----------
start_sofa
pause 800
key Esc
state
c 'Q3 No welcome once set up' '[ "${S[layer]:-}" = none ]' "${S[layer]:-}"
c 'Q4 Settings were kept' '[ "$(setting .settings.controllerLayout)" = ps ]'
kill -TERM "$PID"
c 'Q5 The session ending (SIGTERM) quits' 'gone "$PID" 4'

# ---------- The real world ----------
AFTER=$(fingerprint)
c 'X1 QA touched nothing outside its folder' '[ "$BEFORE" = "$AFTER" ]' "$([ "$BEFORE" != "$AFTER" ] && echo "$BEFORE -> $AFTER")"
ERR=$(grep -hE 'panic|graphics failed|start failed' "$LOG" "$OUT/stderr.txt" 2> /dev/null | head -1)
c 'X2 No errors in the log' '[ -z "$ERR" ]' "$ERR"

cp "$LOG" "$OUT/qa.log" 2> /dev/null || true
SUMMARY="${#RESULTS[@]} checks, $FAILS failed"
RESULTS+=("$SUMMARY")
printf '%s\n' "${RESULTS[@]}" > "$OUT/qa.txt"
echo "$SUMMARY"
[ "$FAILS" = 0 ]
