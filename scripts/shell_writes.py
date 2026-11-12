#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""What a shell command would write, for the PreToolUse hook on Bash (worktree_fresh.py bash-check, AC-104):
redirections, tee, sed -i, perl -i, cp, mv, rm, touch and git's verbs that change a checkout, each with the
folder it runs in. A heuristic that says so: a program writing a file itself passes, and git's commit hook
is the backstop; a target it cannot read (a variable, a substitution) is no target."""
import re

# nothing like these in the command, no parsing and no git process: most commands stop here
WORTH_READING = re.compile(r">|\b(?:tee|sed|perl|cp|mv|rm|touch|git)\b|TM_EDIT_PRIMARY=")
GIT_WRITES = {"add", "apply", "checkout", "cherry-pick", "clean", "commit", "merge", "pull", "rebase", "reset",
              "restore", "revert", "rm", "mv", "stash", "switch"}
GIT_READS = {("stash", "list"), ("stash", "show")}
SEPARATORS = {"&&", "||", ";;", ";", "|&", "|", "&", "(", ")", "\n"}
REDIRECTS = {">>", ">|", "&>>", "&>", ">&", ">"}
INPUTS = {"<", "<<", "<<<", "<&"}
OPERATORS = sorted(SEPARATORS | REDIRECTS | INPUTS, key=len, reverse=True)   # the longest first
# what stands before a command without changing it
PREFIXES = {"env", "command", "exec", "time", "builtin", "nohup", "then", "do", "else", "if", "while", "until", "{", "!"}
ARITHMETIC_OR_TEST = re.compile(r"\(\(.*?\)\)|\[\[.*?\]\]", re.S)   # their > compares
HEREDOC = re.compile(r"<<-?\s*(['\"]?)(\w+)\1[^\n]*\n(?:.*?\n)??[ \t]*\2[ \t]*(?=\n|$)", re.S)
ASSIGNMENT = re.compile(r"[A-Za-z_]\w*=")
SUBSTITUTION = "substitution"   # tokens' mark for the body of a $( ) inside double quotes


def strip_text(command):
    """The command without what is no command: here-documents' bodies, arithmetic, [[ tests ]]."""
    return ARITHMETIC_OR_TEST.sub(" ", HEREDOC.sub(lambda m: m.group(0).split("\n", 1)[0] + "\n", command))


def quoted_end(command, i):
    """The index past the `"` closing the double quote opened at `i`; a `$( )` in it keeps its own quotes."""
    i, n = i + 1, len(command)
    while i < n and command[i] != '"':
        if command.startswith("$(", i):
            i = substitution_end(command, i)
        else:
            i += 2 if command[i] == "\\" else 1
    return i + 1


def substitution_end(command, i):
    """The index past the `)` closing the `$(` at `i`, read as bash reads it: its own quotes, nested ( )."""
    depth, i, n = 1, i + 2, len(command)
    while depth and i < n:
        c = command[i]
        if c == "\\":
            i += 2
        elif c == "'":
            end = command.find("'", i + 1)
            i = n if end < 0 else end + 1
        elif c == '"':
            i = quoted_end(command, i)
        else:
            depth += (c == "(") - (c == ")")
            i += 1
    return i


def tokens(command):
    """[(text, quoted)] of the command's words and operators; an operator is unquoted. A `$( )` inside double
    quotes is its word's text, and after that word its body, marked SUBSTITUTION: a command of its own."""
    out, subs, word, quoted, i, n = [], [], "", False, 0, len(command)

    def flush():
        nonlocal word, quoted, subs
        if word or quoted:
            out.append((word, quoted))
        out.extend(subs)
        word, quoted, subs = "", False, []
    while i < n:
        c = command[i]
        if c == "\\" and i + 1 < n:
            word += command[i + 1]
            quoted, i = True, i + 2
        elif c == "'":
            end = command.find("'", i + 1)
            end = n if end < 0 else end
            word += command[i + 1:end]
            quoted, i = True, end + 1
        elif c == '"':
            i += 1
            while i < n and command[i] != '"':
                if command.startswith("$(", i):
                    end = substitution_end(command, i)
                    subs.append((command[i + 2:end - 1], SUBSTITUTION))
                    word += command[i:end]
                    i = end
                    continue
                if command[i] == "\\" and i + 1 < n:
                    i += 1
                word += command[i]
                i += 1
            quoted, i = True, i + 1
        elif c == "#" and not word and not quoted:
            while i < n and command[i] != "\n":
                i += 1
        elif c in " \t":
            flush()
            i += 1
        else:
            op = next((o for o in OPERATORS if command.startswith(o, i)), None)
            if op is None:
                word += c
                i += 1
                continue
            if op in REDIRECTS and word.isdigit() and not quoted:
                word = ""                 # 2>: the descriptor, no word
            flush()
            out.append((op, False))
            i += len(op)
    flush()
    return out


def simple_commands(words):
    """The commands of a tokenized line: [(arguments, redirection targets, substitutions' bodies)]; here-strings
    and inputs dropped."""
    done, args, targets, subs, i = [], [], [], [], 0
    while i < len(words):
        text, quoted = words[i]
        if quoted is SUBSTITUTION:
            subs.append(text)
        elif not quoted and text in SEPARATORS:
            done.append((args, targets, subs))
            args, targets, subs = [], [], []
        elif not quoted and text in REDIRECTS:
            if i + 1 < len(words):
                target = words[i + 1][0]
                if not (text == ">&" and (target.isdigit() or target == "-")):
                    targets.append(target)
            i += 1
        elif not quoted and text in INPUTS:
            i += 1
        else:
            args.append(text)
        i += 1
    done.append((args, targets, subs))
    return done


def readable(path):
    """Whether a target says where it is: a variable, a substitution or a device says nothing."""
    return bool(path) and not any(c in path for c in "$`") and not path.startswith("/dev/")


def operands(args):
    return [a for a in args if a and not a.startswith("-")]


def edited_files(rest, script_flag):
    """The files of `sed -i` or `perl -i`: its operands but the script, given by `script_flag` or first."""
    names, given, i = [], False, 0
    while i < len(rest):
        if script_flag.fullmatch(rest[i]):
            given, i = True, i + 1            # its value
        elif rest[i] and not rest[i].startswith("-"):
            names.append(rest[i])
        i += 1
    return names if given else names[1:]


def at(folder, path):
    """`path` run in `folder`; None for a relative path in a folder a variable names."""
    if path.startswith(("/", "~")):
        return path
    return f"{folder}/{path}" if folder else None


def writes(command, cwd):
    """(refuses inline, [(path, its own place)], [folder of a git verb]) for this command run in `cwd`.
    `its own place` is True for rm and mv: a link is judged where it lives, not where it points."""
    if not WORTH_READING.search(command):
        return False, [], []
    inline, paths, git_folders, folder = False, [], [], cwd

    def placed(names, own):
        return [(where, own) for name in names if readable(name) and (where := at(folder, name))]
    for args, targets, subs in simple_commands(tokens(strip_text(command))):
        for body in subs:                       # run first, in this folder; a cd in one stays in it
            sub_inline, sub_paths, sub_git = writes(body, folder)
            inline, paths, git_folders = inline or sub_inline, paths + sub_paths, git_folders + sub_git
        paths += placed(targets, False)
        while args and (ASSIGNMENT.match(args[0]) or args[0] in PREFIXES):
            inline = inline or args[0].startswith("TM_EDIT_PRIMARY=")
            args = args[1:]
        if not args:
            continue
        program, rest = args[0].rsplit("/", 1)[-1], args[1:]
        inline = inline or (program == "export" and any(a.startswith("TM_EDIT_PRIMARY=") for a in rest))
        named, own = [], False
        if program in ("cd", "pushd"):
            where = operands(rest)
            folder = at(folder, where[0]) if where and readable(where[0]) else (folder if not where else None)
            continue
        if program in ("tee", "touch", "rm", "mv"):
            named, own = operands(rest), program in ("rm", "mv")
        elif program == "cp":
            named = operands(rest)[-1:]
        elif program == "sed" and any(a.startswith("-i") for a in rest):
            named = edited_files(rest, re.compile(r"-[ef]"))
        elif program == "perl" and any(re.match(r"-\w*i", a) for a in rest):
            named = edited_files(rest, re.compile(r"-\w*e"))
        elif program == "git":
            where, i = folder, 0
            while i < len(rest) and rest[i].startswith("-"):
                if rest[i] == "-C" and i + 1 < len(rest):
                    where = at(folder, rest[i + 1]) if readable(rest[i + 1]) else None
                    i += 1
                elif rest[i] == "-c":
                    i += 1
                i += 1
            verb = rest[i:i + 2]
            if verb and verb[0] in GIT_WRITES and tuple(verb) not in GIT_READS and where and readable(where):
                git_folders.append(where)
        paths += placed(named, own)
    return inline, paths, git_folders
