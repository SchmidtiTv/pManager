#!/usr/bin/env zsh
# End-to-end test of the `pman` binary on Linux (also runs on macOS).
#
# Builds pman, runs the unit/integration tests, then drives the real binary
# against a throw-away sandbox: fake projects with back-dated mtimes, real git
# repositories, and its own ledger and strip profiles. Nothing outside the
# sandbox is touched unless --with-auto-run is given.
#
# Usage: scripts/e2e.zsh [options]
#   --release        build and test the release binary
#   --bin PATH       test an existing pman binary instead of building one
#   --skip-cargo     skip `cargo test`
#   --with-auto-run  also install/remove the real scheduled job (skipped if
#                    one is already installed, so yours is never replaced)
#   --keep           keep the sandbox even when everything passes
#
# Requires: zsh, git, jq (apt install jq / dnf install jq / brew install jq).

emulate -L zsh
setopt pipe_fail no_unset
zmodload zsh/datetime

# ── options ──────────────────────────────────────────────────────────────────
ROOT=${0:A:h:h}
PROFILE=debug
BIN=""
RUN_CARGO=1
WITH_AUTO_RUN=0
KEEP=0
cargo_flags=()

while (( $# )); do
  case $1 in
    --release)       PROFILE=release ;;
    --bin)           BIN=${2:A}; shift ;;
    --skip-cargo)    RUN_CARGO=0 ;;
    --with-auto-run) WITH_AUTO_RUN=1 ;;
    --keep)          KEEP=1 ;;
    -h|--help)       sed -n '2,19p' $0; exit 0 ;;
    *) print -u2 "unknown option: $1"; exit 2 ;;
  esac
  shift
done

for tool in git jq; do
  if ! command -v $tool >/dev/null; then
    print -u2 "error: '$tool' is required (e.g. apt install $tool / brew install $tool)"
    exit 2
  fi
done

# Plain output for greppable logs; no inherited strip dir.
export NO_COLOR=1
unset PMANAGER_STRIP_DIR RUST_LOG

# ── reporting ────────────────────────────────────────────────────────────────
PASSED=0
FAILED=0
FAILURES=()

section() { print -P "\n%B== $1 ==%b" }
ok()      { (( PASSED++ )); print -P "  %F{green}PASS%f $1" }
bad() {
  (( FAILED++ )); FAILURES+=("$1")
  print -P "  %F{red}FAIL%f $1"
  [[ -n ${2:-} ]] && print -r -- "$2" | sed 's/^/       | /'
  return 0
}
skip()    { print -P "  %F{yellow}SKIP%f $1" }

# check DESC CMD...  -- passes when CMD succeeds
check() {
  local desc=$1; shift
  if "$@"; then ok "$desc"; else bad "$desc" "${CTX:-}"; fi
}

# ── build ────────────────────────────────────────────────────────────────────
if [[ -z $BIN ]]; then
  section "build ($PROFILE)"
  [[ $PROFILE == release ]] && cargo_flags=(--release)
  if cargo build --manifest-path $ROOT/Cargo.toml $cargo_flags; then
    ok "cargo build"
  else
    bad "cargo build"; exit 1
  fi
  BIN=$ROOT/target/$PROFILE/pman
fi
[[ -x $BIN ]] || { print -u2 "error: no pman binary at $BIN"; exit 1 }

if (( RUN_CARGO )); then
  section "cargo test"
  if cargo test --manifest-path $ROOT/Cargo.toml $cargo_flags --quiet; then
    ok "cargo test"
  else
    bad "cargo test"
  fi
fi

# ── sandbox ──────────────────────────────────────────────────────────────────
W=$(mktemp -d "${TMPDIR:-/tmp}/pman-e2e.XXXXXX")
W=${W:A}   # resolve symlinks (/var -> /private/var on macOS)
PROJ=$W/projects
STRIP=$W/strip
DB=$W/ledger/pmanager.db
mkdir -p $PROJ $STRIP

finish() {
  if (( FAILED == 0 && KEEP == 0 )); then
    rm -rf $W
  else
    print "\nsandbox kept at: $W"
  fi
}
trap finish EXIT

# pman ARGS...  -> sets OUT (stdout), ERR (stderr), RC; stdin is never a tty.
OUT="" ERR="" RC=0
pman() {
  $BIN "$@" >$W/.out 2>$W/.err </dev/null
  RC=$?
  OUT=$(<$W/.out)
  ERR=$(<$W/.err)
  CTX="exit=$RC"$'\n'"stdout: $OUT"$'\n'"stderr: $ERR"
}
# p ARGS... -> pman against the main sandbox ledger and profiles
p() { pman --db $DB --strip-dir $STRIP "$@" }

succeeds() { (( RC == 0 )) }
fails_with() { (( RC != 0 )) && [[ $ERR == *"$1"* ]] }

# field JSON NAME KEY -> the KEY of the row whose name is NAME
field() { jq -r --arg n $2 --arg k $3 '.[] | select(.name == $n) | .[$k]' <<<$1 }
rows()  { jq 'length' <<<$1 }

# age DIR DAYS -> back-date every file and folder under DIR
age() {
  local ts
  strftime -s ts '%Y%m%d%H%M' $(( EPOCHSECONDS - $2 * 86400 ))
  find $1 -exec touch -t $ts {} +
}

# git_repo DIR -> init with a local identity, commit everything
git_repo() {
  git -C $1 -c init.defaultBranch=main init -q
  git -C $1 config user.name pman-e2e
  git -C $1 config user.email pman-e2e@example.invalid
  git -C $1 config core.autocrlf false
  git -C $1 add -A
  git -C $1 commit -qm seed
}

python_project() {
  mkdir -p $1/.venv/lib
  print '[project]' >$1/pyproject.toml
  print 'print("hi")' >$1/main.py
  head -c 20000 /dev/zero >$1/.venv/lib/big.bin
}
node_project() {
  mkdir -p $1/node_modules/left-pad
  print '{"name":"x"}' >$1/package.json
  print 'console.log(1)' >$1/index.js
  head -c 20000 /dev/zero >$1/node_modules/left-pad/index.js
}

# Strip profiles in the format the README documents.
write_profiles() {
  local dir=$1
  mkdir -p $dir
  cat >$dir/profiles.json <<'EOF'
[
  { "language": "python", "file": "./python.profile.json" },
  { "language": "node",   "file": "./node.profile.json" }
]
EOF
  cat >$dir/python.profile.json <<'EOF'
{
  "language": "python",
  "detection":  [ { "type": "file", "element": "pyproject.toml" },
                  { "type": "folder", "element": ".venv" } ],
  "strippable": [ { "type": "folder", "element": ".venv" } ]
}
EOF
  cat >$dir/node.profile.json <<'EOF'
{
  "language": "node",
  "detection":  [ { "type": "file", "element": "package.json" } ],
  "strippable": [ { "type": "folder", "element": "node_modules" } ]
}
EOF
}
write_profiles $STRIP

is_7z() { [[ $(od -An -tx1 -N6 $1 | tr -d ' \n') == 377abcaf271c ]] }

# ── CLI surface ──────────────────────────────────────────────────────────────
section "CLI surface"
pman --version;          check "--version prints the version" eval '[[ $RC == 0 && $OUT == pman\ * ]]'
pman --help;             check "--help lists every command" eval '[[ $OUT == *auto-run* && $OUT == *list* && $OUT == *project* && $OUT == *run* ]]'
pman bogus;              check "an unknown command is rejected" eval '(( RC != 0 ))'
pman -v -q list;         check "-v and -q conflict" eval '(( RC != 0 ))'
p list -o json;          check "list on a fresh ledger is an empty JSON array" eval '[[ $RC == 0 && $(rows $OUT) == 0 ]]'
check "the ledger file is created on first use" test -f $DB
p list;                  check "list (table) on an empty ledger succeeds" succeeds

# ── project add ──────────────────────────────────────────────────────────────
section "project add"

mkdir -p $PROJ/fresh;        python_project $PROJ/fresh;     age $PROJ/fresh 10
mkdir -p $PROJ/strip-me;     python_project $PROJ/strip-me;  age $PROJ/strip-me 70
mkdir -p $PROJ/no-profile;   print hi >$PROJ/no-profile/readme.txt; age $PROJ/no-profile 70
mkdir -p $PROJ/node-old;     node_project $PROJ/node-old;    age $PROJ/node-old 130

# A clean repo: the strippable folder is ignored, so the tree is not dirty.
mkdir -p $PROJ/git-clean;    node_project $PROJ/git-clean
print 'node_modules/' >$PROJ/git-clean/.gitignore
git_repo $PROJ/git-clean;    age $PROJ/git-clean 130

# Dirty (untracked file): archiving is deferred to 160 days, stripping is not.
mkdir -p $PROJ/git-dirty;    node_project $PROJ/git-dirty
print 'node_modules/' >$PROJ/git-dirty/.gitignore
git_repo $PROJ/git-dirty;    print wip >$PROJ/git-dirty/notes.txt
age $PROJ/git-dirty 130

mkdir -p $PROJ/git-dirty-old; print hi >$PROJ/git-dirty-old/readme.txt
git_repo $PROJ/git-dirty-old; print wip >$PROJ/git-dirty-old/notes.txt
age $PROJ/git-dirty-old 170

# .pmanagerignore: a fresh file under an ignored folder is not activity.
mkdir -p $PROJ/ignored/build; print hi >$PROJ/ignored/readme.txt
print 'build/' >$PROJ/ignored/.pmanagerignore
print out >$PROJ/ignored/build/out.bin; age $PROJ/ignored 90; touch $PROJ/ignored/build/out.bin
# Same tree without the ignore file: the fresh file makes it active.
mkdir -p $PROJ/not-ignored/build; print hi >$PROJ/not-ignored/readme.txt
print out >$PROJ/not-ignored/build/out.bin; age $PROJ/not-ignored 90; touch $PROJ/not-ignored/build/out.bin

mkdir -p $PROJ/vanished;     print hi >$PROJ/vanished/readme.txt

ALL=(fresh strip-me no-profile node-old git-clean git-dirty git-dirty-old ignored not-ignored vanished)
for n in $ALL; do
  p project add $PROJ/$n --name $n
  check "add $n" succeeds
done

p list -o json; LIST=$OUT
check "list shows all ${#ALL} projects" eval '[[ $(rows $LIST) == ${#ALL} ]]'
check "every new project is 'registered'" eval '[[ $(jq -r "[.[].state] | unique | join(\",\")" <<<$LIST) == registered ]]'
check "idle days come from the back-dated mtimes" eval '[[ $(field $LIST strip-me idle) == 70 ]]'
typeset -A PATHS
for n in $ALL; do PATHS[$n]=$(field $LIST $n path); done
check "stored paths are absolute and canonical" eval '[[ ${PATHS[fresh]} == $PROJ/fresh ]]'

p project add $PROJ/fresh/../fresh/. --name other
check "the same folder spelled differently is a duplicate" fails_with "already registered"
p project add $PROJ/no-profile/../vanished --name vanished-2
check "a path already registered is refused" fails_with "already registered"
mkdir -p $PROJ/spare
p project add $PROJ/spare --name fresh
check "a name already in use is refused" fails_with "already used"
p project add $PROJ/spare --name "   "
check "a blank name is refused" fails_with "must not be empty"
p project add $PROJ/spare
check "no --name without a terminal asks for --name" fails_with "pass --name"
p project add $PROJ/does-not-exist --name ghost
check "a missing folder is refused" fails_with "could not resolve"
print x >$PROJ/a-file
p project add $PROJ/a-file --name file
check "a file is not a project" fails_with "is not a directory"
p list -o json
check "failed adds left the ledger unchanged" eval '[[ $(rows $OUT) == ${#ALL} ]]'

rm -rf $PROJ/vanished

# ── strip profiles ───────────────────────────────────────────────────────────
section "strip profile validation"
pman --db $DB --strip-dir $W/no-such-dir run --dry-run
check "a missing strip directory fails the run" eval '(( RC != 0 ))'

mkdir -p $W/strip-bad; write_profiles $W/strip-bad
sed -i.bak 's/"language": "node"/"language": "deno"/' $W/strip-bad/node.profile.json
pman --db $DB --strip-dir $W/strip-bad run
check "a profile whose language disagrees with the index fails the run" fails_with "invalid"
check "...and nothing was stripped" test -d $PROJ/strip-me/.venv

# ── run --dry-run ────────────────────────────────────────────────────────────
section "run --dry-run"
p run --dry-run -o json; DRY=$OUT
check "dry run succeeds" eval '[[ $RC == 0 ]]'

typeset -A WANT_ACTION WANT_RESULT
WANT_ACTION=(fresh none strip-me strip no-profile none node-old strip+archive git-clean strip+archive
             git-dirty strip git-dirty-old archive ignored none not-ignored none vanished none)
WANT_RESULT=(fresh active strip-me dry-run no-profile "no profile" node-old dry-run git-clean dry-run
             git-dirty dry-run git-dirty-old dry-run ignored "no profile" not-ignored active vanished missing)
for n in $ALL; do
  check "dry run: $n -> ${WANT_ACTION[$n]} / ${WANT_RESULT[$n]}" \
    eval '[[ $(field $DRY $n action) == ${WANT_ACTION[$n]} && $(field $DRY $n result) == ${WANT_RESULT[$n]} ]]'
done
check "dry run frees nothing" eval '[[ $(jq "[.[].freed] | add" <<<$DRY) == 0 ]]'
check ".pmanagerignore keeps build/ out of the idle clock" eval '(( $(field $DRY ignored idle) >= 89 ))'
check "without it, the fresh build file counts" eval '(( $(field $DRY not-ignored idle) == 0 ))'
check "dry run deleted nothing" eval '[[ -d $PROJ/strip-me/.venv && -d $PROJ/node-old/node_modules && -d $PROJ/git-dirty-old ]]'
check "dry run wrote no archive" eval '[[ -z $(print -l $PROJ/*.pproject(N)) ]]'
p list -o json
check "dry run changed no state" eval '[[ $(jq -r "[.[].state] | unique | join(\",\")" <<<$OUT) == registered ]]'

PMANAGER_STRIP_DIR=$STRIP pman --db $DB run --dry-run -o json
check "PMANAGER_STRIP_DIR is honoured" eval '[[ $RC == 0 && $(rows $OUT) == ${#ALL} ]]'

p run --dry-run
check "dry run renders as a table" eval '[[ $RC == 0 && $OUT == *NAME* && $OUT == *ACTION* ]]'

# ── run ──────────────────────────────────────────────────────────────────────
section "run"
p run -o json; RUN=$OUT
check "run succeeds" eval '[[ $RC == 0 ]]'
for n in $ALL; do
  want=${WANT_RESULT[$n]}; [[ $want == dry-run ]] && want=ok
  check "run: $n -> ${WANT_ACTION[$n]} / $want" \
    eval '[[ $(field $RUN $n action) == ${WANT_ACTION[$n]} && $(field $RUN $n result) == $want ]]'
done

check "strip: .venv removed"                  test ! -e $PROJ/strip-me/.venv
check "strip: sources kept"                   eval '[[ -f $PROJ/strip-me/main.py && -f $PROJ/strip-me/pyproject.toml ]]'
check "strip: freed bytes reported"           eval '(( $(field $RUN strip-me freed) >= 20000 ))'
check "untouched project kept its .venv"      test -d $PROJ/fresh/.venv
check "no-profile project untouched"          test -f $PROJ/no-profile/readme.txt
for n in node-old git-clean git-dirty-old; do
  check "archive: $n folder removed"          test ! -e $PROJ/$n
  check "archive: $n.pproject is a 7z file"   eval '[[ -f $PROJ/$n.pproject ]] && is_7z $PROJ/$n.pproject'
  check "archive: no .part left for $n"       test ! -e $PROJ/$n.pproject.part
done
check "dirty repo: stripped but not archived" eval '[[ -d $PROJ/git-dirty && ! -e $PROJ/git-dirty/node_modules ]]'
check "dirty repo: uncommitted work kept"     test -f $PROJ/git-dirty/notes.txt

p list -o json; LIST=$OUT
typeset -A WANT_STATE
WANT_STATE=(fresh registered strip-me stripped no-profile registered node-old archived git-clean archived
            git-dirty stripped git-dirty-old archived ignored registered not-ignored registered vanished registered)
for n in $ALL; do
  check "state: $n is ${WANT_STATE[$n]}" eval '[[ $(field $LIST $n state) == ${WANT_STATE[$n]} ]]'
done

section "second run"
p run -o json; RUN2=$OUT
check "second run succeeds" succeeds
check "archived projects are left alone"  eval '[[ $(field $RUN2 node-old result) == "already archived" ]]'
check "stripped project is not re-stripped" eval '[[ $(field $RUN2 strip-me action) == none && $(field $RUN2 strip-me result) == active ]]'
check "a gone folder is reported missing" eval '[[ $(field $RUN2 vanished result) == missing ]]'
check "second run frees nothing" eval '[[ $(jq "[.[].freed] | add" <<<$RUN2) == 0 ]]'

# ── failure isolation ────────────────────────────────────────────────────────
section "one failing project does not stop the pass"
DB2=$W/ledger/failing.db
mkdir -p $W/iso/bad-ignore $W/iso/sibling
print hi >$W/iso/bad-ignore/readme.txt
node_project $W/iso/sibling
pman --db $DB2 --strip-dir $STRIP project add $W/iso/bad-ignore --name bad-ignore
pman --db $DB2 --strip-dir $STRIP project add $W/iso/sibling --name sibling
print 'build/{' >$W/iso/bad-ignore/.pmanagerignore   # broken after registration
age $W/iso 70
pman --db $DB2 --strip-dir $STRIP run -o json; ISO=$OUT
check "the run exits non-zero"                eval '(( RC != 0 ))'
check "the error names .pmanagerignore"       eval '[[ $ERR == *.pmanagerignore* ]]'
check "the broken project's row says failed"  eval '[[ $(field $ISO bad-ignore result) == failed ]]'
check "its sibling was still stripped"        eval '[[ $(field $ISO sibling result) == ok && ! -e $W/iso/sibling/node_modules ]]'

# ── remove & cleanup ─────────────────────────────────────────────────────────
section "project remove"
p project remove ${PATHS[node-old]}
check "remove an archived project" succeeds
check "...warns that its archive is left in place" eval '[[ $ERR == *"left in place"* ]]'
check "...and leaves the .pproject on disk" test -f $PROJ/node-old.pproject
p project remove ${PATHS[node-old]}
check "removing it again fails" fails_with "no project is registered"
p project remove $PROJ/fresh/../fresh
check "remove accepts any spelling of an existing folder" succeeds
check "...and never deletes the project's files" test -f $PROJ/fresh/main.py
p project remove $PROJ/never-registered
check "removing an unknown path fails" fails_with "no project is registered"

section "project cleanup"
rm -rf $PROJ/no-profile
p project cleanup -o json; CLEAN=$OUT
check "cleanup succeeds" succeeds
check "cleanup removes a vanished registered project" eval '[[ $(field $CLEAN vanished result) == removed ]]'
check "cleanup removes a deleted project"             eval '[[ $(field $CLEAN no-profile result) == removed ]]'
check "cleanup keeps archived rows"                   eval '[[ $(field $CLEAN git-clean result) == kept ]]'
check "cleanup keeps existing projects"               eval '[[ $(field $CLEAN strip-me result) == kept ]]'
p list -o json
check "list reflects remove + cleanup" eval '[[ $(rows $OUT) == $(( ${#ALL} - 4 )) ]]'
p project cleanup -o json
check "a second cleanup removes nothing" eval '[[ $RC == 0 && $(jq "[.[] | select(.result == \"removed\")] | length" <<<$OUT) == 0 ]]'

# ── auto-run ─────────────────────────────────────────────────────────────────
section "auto-run"
case $OSTYPE in
  linux*)  JOB_FILE=${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/pman-run.timer ;;
  darwin*) JOB_FILE=$HOME/Library/LaunchAgents/pmanager.run.plist ;;
  *)       JOB_FILE="" ;;
esac
HAD_JOB=0; [[ -n $JOB_FILE && -e $JOB_FILE ]] && HAD_JOB=1

pman --db $DB --strip-dir $W/no-such-dir auto-run add
check "auto-run add refuses broken strip profiles" eval '(( RC != 0 ))'
if (( ! HAD_JOB )) && [[ -n $JOB_FILE ]]; then
  check "...before touching the scheduler" test ! -e $JOB_FILE
fi

if (( ! WITH_AUTO_RUN )); then
  skip "scheduler install/remove (pass --with-auto-run)"
elif [[ -z $JOB_FILE ]]; then
  skip "no scheduler backend on $OSTYPE"
elif (( HAD_JOB )); then
  skip "a pman job is already installed at $JOB_FILE; not replacing it"
elif [[ $OSTYPE == linux* ]] && ! systemctl --user show-environment >/dev/null 2>&1; then
  skip "no systemd user session (container/WSL?)"
else
  p auto-run add
  check "auto-run add installs the job" eval '[[ $RC == 0 && $ERR == *installed* ]]'
  check "...writing $JOB_FILE" test -f $JOB_FILE
  if [[ $OSTYPE == linux* ]]; then
    check "...and the timer is enabled" eval 'systemctl --user is-enabled --quiet pman-run.timer'
  fi
  p auto-run add
  check "adding it again changes nothing" eval '[[ $RC == 0 && $ERR == *"already installed"* ]]'
  p auto-run remove
  check "auto-run remove removes it" eval '[[ $RC == 0 && $ERR == *removed* && ! -e $JOB_FILE ]]'
  p auto-run remove
  check "removing it again is a quiet no-op" eval '[[ $RC == 0 && $ERR == *"no scheduled run"* ]]'
fi

# ── summary ──────────────────────────────────────────────────────────────────
print -P "\n%B$PASSED passed, $FAILED failed%b"
for f in $FAILURES; do print -P "  %F{red}✗%f $f"; done
(( FAILED == 0 ))
