;;; samu.el --- samudo from Emacs -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Marc Seibert
;; SPDX-License-Identifier: AGPL-3.0-or-later

;; Author: Marc Seibert
;; Version: 0.1.0
;; Package-Requires: ((emacs "28.1"))
;; Homepage: https://github.com/clarknovalis/samudo

;;; Commentary:

;; samudo, the task manager, from Emacs (ADR 0008 §4, ADR 0082).
;; Everything goes through the command line `samu' with `--format
;; json': its configuration, its session and its server, the local
;; replica (ADR 0066) included.  Emacs writes as the person signed in
;; there.  A key that changes something waits for its call; what the
;; buffers show is fetched in the background, and only for buffers in
;; a window.
;;
;;   (load "~/path/to/samudo/emacs/samu.el")
;;   (global-set-key (kbd "C-c s") #'samu)          ; the forecast
;;   (global-set-key (kbd "C-c S") #'samu-capture)  ; quick capture
;;   (samu-clock-mode-line-mode 1)                  ; the running clock
;;
;; Views: `samu' (forecast), `samu-inbox', `samu-available',
;; `samu-today', `samu-flagged', `samu-project', `samu-tag',
;; `samu-search', `samu-projects'.  `samu-show' opens one task.
;;
;; Keys in a view and in a task's buffer, those of the web UI with the
;; navigation of Emacs (`?' lists them all):
;;
;;   n p j k   next / previous task      RET TAB   show / show elsewhere
;;   x         done / open again         s t       next state / pick one
;;   f         flag                      c I O     clock toggle / in / out
;;   m C-c C-w move                      e         edit by marks
;;   ,         priority                  :         tags
;;   C-c C-s   planned                   C-c C-d   due
;;   a A       capture / capture below   /         search
;;   X         drop                      DEL       delete
;;   w         copy the samu: link       S         scope
;;   M u U B   mark, unmark, unmark all, act on the marked
;;   h h, h d  habit: done (+1)         h f       habit: not done, relapse
;;   h p       habit: pause             h u       habit: delete the last entry
;;   h s       habit: streak, rate and the last weeks
;;   g q ?     reload, quit, help
;;
;; In a task's buffer also: T title, N note, C comment, + attach,
;; o open a property, l logbook.
;;
;; `samu:<uuid>' is an Org link type.  Following one opens the object
;; here; with `samu-link-target' set to `browser', or with C-u, in the
;; web UI at `samu-web-url'/o/<uuid> (ADR 0075).

;;; Code:

(require 'subr-x)
(require 'cl-lib)
(require 'json)
(require 'parse-time)
(require 'text-property-search)

(declare-function org-link-set-parameters "ol")
(declare-function org-link-store-props "ol")
(declare-function org-read-date "org")
(declare-function evil-set-initial-state "evil-core")

;;;; Settings

(defgroup samu nil
  "samudo, the task manager, from Emacs."
  :group 'external
  :prefix "samu-")

(defcustom samu-executable "samu"
  "The command line of samudo."
  :type 'string)

(defcustom samu-server nil
  "The server, passed as `--server'; nil for what the CLI is set to."
  :type '(choice (const :tag "As the CLI is set" nil) string))

(defcustom samu-account nil
  "The account, passed as `--account'; nil for what the CLI is set to."
  :type '(choice (const :tag "As the CLI is set" nil) string))

(defcustom samu-profile nil
  "The profile, passed as `--profile' (ADR 0086); nil for the CLI's default.
A profile is an account of this computer with its service on the
loopback; `samu-switch-profile' chooses one."
  :type '(choice (const :tag "As the CLI is set" nil) string))

(defcustom samu-scope nil
  "The scope the views look through.
nil is the one chosen for the CLI on this device, `none' no scope,
a string the title of a scope.  `samu-choose-scope' sets it."
  :type '(choice (const :tag "As the CLI chose" nil)
                 (const :tag "No scope" none)
                 (string :tag "Scope")))

(defcustom samu-web-url nil
  "The web UI a `samu:' link opens in the browser and points at when exported.
Like \"https://samu.example.org\"; not needed with `samu-profile'."
  :type '(choice (const :tag "Not set" nil) (string :tag "URL")))

(defcustom samu-link-target 'emacs
  "Where following a `samu:' link opens the object; C-u takes the other."
  :type '(choice (const :tag "Here, in Emacs" emacs)
                 (const :tag "In the web UI" browser)))

(defcustom samu-capture-link t
  "Whether a capture from a file's buffer notes where it came from."
  :type 'boolean)

(defcustom samu-clock-interval 60
  "Seconds between two updates of the running clock in the mode line.
Without `samu-live' each also asks the CLI for the clock."
  :type 'integer)

(defcustom samu-live t
  "Whether open samu buffers and the clock follow changes as they happen.
One process `samu events' runs while a samu buffer is open or the
clock is in the mode line.  `samu-toggle-live' switches it."
  :type 'boolean)

(defface samu-section '((t :inherit org-agenda-structure))
  "A section of a view: a day, a project.")
(defface samu-state '((t :inherit org-todo))
  "The state of a task.")
(defface samu-priority '((t :inherit org-priority))
  "The priority of a task.")
(defface samu-flag '((t :inherit warning))
  "The flag of a task.")
(defface samu-unavailable '((t :inherit shadow))
  "A task that is not available.")
(defface samu-done '((t :inherit org-done))
  "A completed or dropped task.")
(defface samu-overdue '((t :inherit error))
  "The date of an overdue task.")
(defface samu-due-soon '((t :inherit warning))
  "The date of a task due soon.")
(defface samu-meta '((t :inherit shadow))
  "Project, tags and why, behind the title.")
(defface samu-field '((t :inherit font-lock-keyword-face))
  "The name of a field in a task's buffer.")
(defface samu-marked '((t :inherit warning :weight bold))
  "The mark of a marked task.")
(defface samu-title '((t :inherit org-level-1))
  "The title of a view or a task's buffer.")
(defface samu-habit-met '((t :inherit success))
  "A habit whose goal is met, a day done, or a habit to avoid kept.")
(defface samu-habit-partial '((t :inherit warning))
  "A day of a habit with less than its goal.")
(defface samu-habit-fail '((t :inherit error))
  "A day of a habit not done on purpose, or a relapse.")
(defface samu-habit-quiet '((t :inherit shadow))
  "A habit not yet done today, a pause, a day without entry, the streak.")

(defcustom samu-habit-streak-mark "🔥"
  "What stands before the streak of a habit in a row, as in the web and app."
  :type 'string)

;;;; Calling the CLI

(defconst samu--exit-not-found 3
  "The exit code of the CLI when what was asked for does not exist.")

(defconst samu--exit-needs-choice 7
  "The exit code of the CLI when tasks have open tasks below (ADR 0128).")

(define-error 'samu-error "samu")

(defun samu--global-args ()
  "Arguments every call takes: profile, server and account."
  (append (and samu-profile (list "--profile" samu-profile))
          (and samu-server (list "--server" samu-server))
          (and samu-account (list "--account" samu-account))))

(defun samu--scope-args ()
  "Arguments for the scope of the views."
  (pcase samu-scope
    ('nil nil)
    ('none (list "--no-scope"))
    (scope (list "--scope" scope))))

(defvar samu--answers nil
  "While `samu--fetch-async' runs a fetch: the answers it has so far.
A hash table from (ARGS INPUT) to (CODE OUT ERR).  `samu--run' then
answers from it and starts nothing.")

(defvar samu--missing nil
  "The calls a fetch made that `samu--answers' has no answer for.")

(defun samu--run (args &optional input)
  "Run the CLI with ARGS, INPUT on stdin; return (CODE OUT ERR).
Emacs waits for it.  Inside `samu--fetch-async' nothing runs: the
answer comes from `samu--answers', and a call without one is noted
in `samu--missing' and answered as if the CLI had printed nothing."
  (if samu--answers
      (let ((key (list args input)))
        (or (gethash key samu--answers)
            (progn (cl-pushnew key samu--missing :test #'equal)
                   (list 0 "" ""))))
    (let ((err (make-temp-file "samu-err")))
      (unwind-protect
          (with-temp-buffer
            (let* ((coding-system-for-read 'utf-8)
                   (coding-system-for-write 'utf-8)
                   (all (append (samu--global-args) args))
                   (code (if input
                             (apply #'call-process-region input nil samu-executable
                                    nil (list t err) nil all)
                           (apply #'call-process samu-executable nil (list t err)
                                  nil all))))
              (list code (buffer-string)
                    (with-temp-buffer
                      (insert-file-contents err)
                      (string-trim (buffer-string))))))
        (delete-file err)))))

(defun samu--parse (text)
  "TEXT read as JSON: objects as alists, arrays as lists, null and false nil."
  (if (string-blank-p text)
      nil
    (json-parse-string text :object-type 'alist :array-type 'list
                       :null-object nil :false-object nil)))

(defun samu--fail (code err)
  "Signal a failure of the CLI with exit CODE and stderr ERR."
  (let ((last (car (last (split-string err "\n" t)))))
    (signal 'samu-error
            (list (if last
                      (if (string-prefix-p "samu: " last) last (concat "samu: " last))
                    (format "samu: exit %s" code))))))

(defun samu--call-text (args &optional input)
  "Run the CLI with ARGS and INPUT; return its stdout as text."
  (pcase-let ((`(,code ,out ,err) (samu--run args input)))
    (unless (eql code 0) (samu--fail code err))
    out))

(defun samu--call (args &optional input not-found-ok)
  "Run the CLI with ARGS and `--format json'; return the parsed output.
INPUT goes to stdin.  With NOT-FOUND-OK, exit code 3 returns nil."
  (pcase-let ((`(,code ,out ,err)
               (samu--run (append args (list "--format" "json")) input)))
    (cond ((eql code 0) (samu--parse out))
          ((and not-found-ok (eql code samu--exit-not-found)) nil)
          (t (samu--fail code err)))))

(defun samu--get (object &rest keys)
  "The value under KEYS, one after another, in the alist OBJECT."
  (while (and keys object)
    (setq object (and (listp object) (alist-get (pop keys) object))))
  object)

(defun samu--text (value)
  "VALUE as a string, empty for nil."
  (cond ((null value) "")
        ((stringp value) value)
        (t (format "%s" value))))

;;;; Asking without waiting

;; Writing waits for the CLI: one call, and its error belongs to the
;; key just pressed.  Reading does not: what a buffer shows takes
;; several calls, and Emacs stays usable while they run (ADR 0082 §1).

(defvar samu--sync nil
  "Non-nil makes every call of the CLI wait for its answer.
The tests set it: they answer in place of the CLI.")

(defconst samu--max-processes 6
  "How many calls of the CLI run side by side; more wait their turn.")

(defconst samu--max-rounds 8
  "How often a fetch runs again for answers before it counts as failed.")

(defvar samu--running 0
  "How many calls of the CLI run without Emacs waiting.")

(defvar samu--waiting nil
  "The calls waiting for their turn, oldest first: (COMMAND INPUT CALLBACK).")

(defun samu--run-async (args input callback)
  "Run the CLI with ARGS and INPUT on stdin without waiting for it.
CALLBACK gets (CODE OUT ERR), what `samu--run' returns, once it ended."
  (if samu--sync
      (funcall callback (samu--run args input))
    (setq samu--waiting
          (nconc samu--waiting
                 (list (list (cons samu-executable (append (samu--global-args) args))
                             input callback))))
    (samu--run-waiting)))

(defun samu--run-waiting ()
  "Start waiting calls while fewer than `samu--max-processes' run."
  (while (and samu--waiting (< samu--running samu--max-processes))
    (apply #'samu--spawn (pop samu--waiting))))

(defun samu--error-text (err)
  "What to show for the error ERR: the CLI's own line for a `samu-error'."
  (if (and (eq (car err) 'samu-error) (stringp (cadr err)))
      (cadr err)
    (error-message-string err)))

(defun samu--answer (callback answer)
  "Give ANSWER to CALLBACK; an error there becomes a message."
  (condition-case err
      (funcall callback answer)
    (error (message "%s" (samu--error-text err)))))

(defun samu--spawn (command input callback)
  "Start COMMAND with INPUT on stdin; CALLBACK gets (CODE OUT ERR)."
  (let ((out (generate-new-buffer " *samu-out*" t))
        (err (generate-new-buffer " *samu-err*" t)))
    (cl-incf samu--running)
    (condition-case failure
        ;; stderr through a pipe of its own with a sentinel that does
        ;; nothing: the default one writes "Process finished" into it
        (let* ((errors (make-pipe-process :name "samu-err" :buffer err
                                          :noquery t :coding 'utf-8
                                          :sentinel #'ignore))
               (process (make-process
                         :name "samu" :command command :buffer out
                         :stderr errors :connection-type 'pipe
                         :noquery t :coding 'utf-8
                         :sentinel (lambda (process _event)
                                     (unless (process-live-p process)
                                       (samu--finish process errors callback))))))
          ;; it may have ended already; then the sentinel answers
          (ignore-errors
            (when input (process-send-string process input))
            (process-send-eof process)))
      (error
       (kill-buffer out)
       (kill-buffer err)
       (setq samu--running (max 0 (1- samu--running)))
       ;; later, so a caller never gets its answer before it returns
       (run-at-time 0 nil #'samu--answer callback
                    (list 127 "" (error-message-string failure)))))))

(defun samu--finish (process errors callback)
  "PROCESS ended, its stderr in ERRORS: give CALLBACK (CODE OUT ERR)."
  (while (accept-process-output errors 0))
  (let ((answer (list (process-exit-status process)
                      (with-current-buffer (process-buffer process) (buffer-string))
                      (with-current-buffer (process-buffer errors)
                        (string-trim (buffer-string))))))
    (kill-buffer (process-buffer errors))
    (kill-buffer (process-buffer process))
    (setq samu--running (max 0 (1- samu--running)))
    (samu--run-waiting)
    (samu--answer callback answer)))

(defun samu--fetch-async (fetch done &optional fail answers round)
  "Call DONE with what FETCH returns, without waiting for the CLI.
FETCH is plain code that asks the CLI through `samu--run' and returns
what it found; it changes nothing.  It runs once to see what it asks,
those calls run side by side, and it runs again with their answers,
until it has an answer for every call.  An error in a run that still
misses answers does not count.

If FETCH fails with every answer there, the error is shown as a
message and FAIL gets it.  ANSWERS and ROUND are for the next run.
With `samu--sync', FETCH runs once and waits, and its error is
signaled after FAIL."
  (if samu--sync
      (funcall done (condition-case err
                        (funcall fetch)
                      (error (when fail (funcall fail err))
                             (signal (car err) (cdr err)))))
    (let* ((answers (or answers (make-hash-table :test #'equal)))
           (round (or round 1))
           (missing nil)
           (result (let ((samu--answers answers)
                         (samu--missing nil))
                     (prog1 (condition-case err
                                (cons 'ok (funcall fetch))
                              (error (cons 'error err)))
                       (setq missing samu--missing)))))
      (cond
       ((and missing (< round samu--max-rounds))
        (let ((left (length missing)))
          (dolist (key missing)
            (samu--run-async
             (car key) (cadr key)
             (lambda (answer)
               (puthash key answer answers)
               (when (zerop (cl-decf left))
                 (samu--fetch-async fetch done fail answers (1+ round))))))))
       ((and (null missing) (eq (car result) 'ok))
        (funcall done (cdr result)))
       (t
        (let ((err (if missing
                       '(samu-error "samu: this view never stops asking")
                     (cdr result))))
          (message "%s" (samu--error-text err))
          (when fail (funcall fail err))))))))

;;;; What completion offers

(defvar samu--cache nil
  "Projects, tags and templates as the CLI gave them, by kind.")

(defvar samu--note-formats (make-hash-table :test #'equal)
  "The format of each note and comment asked for, by its text.")

(defun samu--forget ()
  "Forget what completion offers, so the next use asks again."
  (setq samu--cache nil)
  (clrhash samu--note-formats))

(defun samu--cached (kind args)
  "The records of KIND, from the CLI with ARGS the first time."
  (or (alist-get kind samu--cache)
      (let ((records (samu--call args)))
        (setf (alist-get kind samu--cache) records)
        records)))

(defun samu--projects ()
  "Every project with its availability."
  (samu--cached 'projects '("projects" "--no-scope")))

(defun samu--tags ()
  "Every tag, with path, key and marks."
  (samu--cached 'tags '("tag" "list")))

(defun samu--templates ()
  "Every capture template."
  (samu--cached 'templates '("template" "list")))

(defun samu--workflow-paths ()
  "The paths of the workflows."
  (cl-loop for tag in (samu--tags)
           when (string-match-p "\\_<workflow\\_>" (samu--text (samu--get tag 'marks)))
           collect (samu--get tag 'path)))

(defun samu--plain-tag-paths ()
  "The paths of the tags that are neither workflows nor their states."
  (let ((workflows (samu--workflow-paths)))
    (cl-loop for tag in (samu--tags)
             for path = (samu--get tag 'path)
             unless (or (member path workflows)
                        (cl-some (lambda (w) (string-prefix-p (concat w "/") path))
                                 workflows))
             collect path)))

(defun samu--tag-path (id)
  "The path of the tag ID, or nil."
  (cl-loop for tag in (samu--tags)
           when (equal (samu--get tag 'id) id) return (samu--get tag 'path)))

(defun samu--project-title (id)
  "The title of the project ID, or nil."
  (cl-loop for p in (samu--projects)
           when (equal (samu--get p 'id) id) return (samu--get p 'title)))

(defun samu--read-project (prompt &optional inbox)
  "Read a project with PROMPT; its id, or \"inbox\" if INBOX allows it."
  (let* ((table (mapcar (lambda (p) (cons (samu--get p 'title) (samu--get p 'id)))
                        (samu--projects)))
         (table (if inbox (cons (cons "Inbox" "inbox") table) table))
         (choice (completing-read prompt table nil t)))
    (cdr (assoc choice table))))

;;;; Rows

(defvar-local samu--rows nil
  "The rows the buffer shows, as the CLI gave them.")

(defvar-local samu--marked nil
  "The ids of the marked tasks.")

(defvar-local samu--view nil
  "How a view fills itself: a plist with :title, :fetch and :project.
:fetch returns a list of (HEADING . ROWS); :project shows the project.")

(defvar-local samu--task-id nil
  "The id of the task or project a task's buffer shows.")

(defvar-local samu--task-row nil
  "The row of `samu list' of the buffer's task, or nil for a project.")

(defvar-local samu--task-object nil
  "The object of the buffer's task as `samu show' gave it.")

(defvar-local samu--habits nil
  "The open habits among the rows, by task id, as `samu habit list' gave them.
Nil when no row is a habit.  Fetched once each time the buffer is filled.")

(defvar-local samu--habit-id nil
  "The id of the habit a habit's buffer shows (`samu-habit-show').")

(defun samu--row-task (row)
  "The task in a row of `samu list'."
  (samu--get row 'status 'task))

(defun samu--row-closed-p (row)
  "Whether the task of ROW is completed or dropped."
  (let ((task (samu--row-task row)))
    (and (or (samu--get task 'completed) (samu--get task 'dropped)) t)))

(defun samu--parent (row)
  "The id of the parent task of ROW, or nil."
  (samu--get (samu--row-task row) 'parent))

(defun samu--depth (row ids)
  "How many ancestors of ROW are among the rows in IDS, a hash table."
  (let ((depth 0) (parent (samu--parent row)) (seen nil))
    (while (and parent (gethash parent ids) (not (member parent seen)))
      (push parent seen)
      (setq depth (1+ depth)
            parent (samu--parent (gethash parent ids))))
    depth))

(defun samu--date-face (due)
  "The face for a date whose task is DUE: none, soon or overdue."
  (pcase due
    ("overdue" 'samu-overdue)
    ("soon" 'samu-due-soon)
    (_ 'samu-meta)))

(defun samu--row-line (row depth &optional project)
  "The line for ROW, indented by DEPTH; PROJECT non-nil shows the project."
  (let* ((marked (member (samu--get row 'id) samu--marked))
         (habit (and samu--habits (gethash (samu--get row 'id) samu--habits)))
         (state (samu--text (samu--get row 'state)))
         (priority (samu--text (samu--get row 'priority)))
         (flagged (not (string-empty-p (samu--text (samu--get row 'flag)))))
         (closed (samu--row-closed-p row))
         (available (or (samu--get row 'available) (null (assq 'available row))))
         (project-title (samu--text (samu--get row 'project)))
         (tags (samu--text (samu--get row 'tags)))
         (date (samu--text (samu--get row 'due_text)))
         (why (samu--text (samu--get row 'why))))
    (concat
     (if marked (propertize "> " 'face 'samu-marked) "  ")
     (make-string (* 2 depth) ?\s)
     (if (and habit (not closed)) (concat (samu--habit-cell habit) " ") "")
     (if (string-empty-p state) "" (concat (propertize state 'face 'samu-state) " "))
     (if (string-empty-p priority) ""
       (concat (propertize (concat "!" priority) 'face 'samu-priority) " "))
     (if flagged (concat (propertize "⚑" 'face 'samu-flag) " ") "")
     (propertize (samu--text (samu--get row 'title))
                 'face (cond (closed 'samu-done)
                             ((not available) 'samu-unavailable)))
     (if (and project (not (string-empty-p project-title)))
         (propertize (concat "  #" project-title) 'face 'samu-meta)
       "")
     (if (string-empty-p tags) "" (propertize (concat "  " tags) 'face 'samu-meta))
     (if (string-empty-p date) ""
       (propertize (concat "  " date) 'face (samu--date-face (samu--get row 'due))))
     (if (string-empty-p why) "" (propertize (concat "  (" why ")") 'face 'samu-meta)))))

(defun samu--insert-rows (rows &optional project)
  "Insert ROWS, children below their parents; PROJECT shows the project."
  (let ((ids (make-hash-table :test #'equal)))
    (dolist (row rows) (puthash (samu--get row 'id) row ids))
    (dolist (row rows)
      (let ((start (point)))
        (insert (samu--row-line row (samu--depth row ids) project) "\n")
        (put-text-property start (point) 'samu-row row)
        (when (samu--get row 'id)
          (put-text-property start (point) 'samu-id (samu--get row 'id)))))))

(defun samu--scope-label ()
  "How the header names the scope."
  (pcase samu-scope
    ('nil "Scope as the CLI")
    ('none "No scope")
    (scope (concat "Scope " scope))))

(defun samu--fetch-view ()
  "What a view shows, from the CLI: a plist with :groups, :rows and :habits."
  (let* ((groups (funcall (plist-get samu--view :fetch)))
         (rows (apply #'append (mapcar #'cdr groups))))
    (list :groups groups
          :rows rows
          :habits (samu--habit-statuses rows (samu--scope-args)))))

(defun samu--draw-view (data)
  "Fill a view from DATA of `samu--fetch-view', point on the same task."
  (let ((inhibit-read-only t)
        (id (get-text-property (point) 'samu-id))
        (line (line-number-at-pos))
        (groups (plist-get data :groups)))
    (setq samu--rows (plist-get data :rows)
          samu--habits (plist-get data :habits))
    (erase-buffer)
    (insert (propertize (plist-get samu--view :title) 'face 'samu-title)
            (propertize (concat "  " (samu--scope-label)) 'face 'samu-meta)
            "\n")
    (if (null samu--rows)
        (insert "\n  (nothing)\n")
      (dolist (group groups)
        (when (cdr group)
          (when (car group)
            (insert "\n" (propertize (car group) 'face 'samu-section) "\n"))
          (samu--insert-rows (cdr group) (plist-get samu--view :project)))))
    (samu--goto id line)))

(defun samu--fetch ()
  "What this samu buffer shows, from the CLI; `samu--draw' takes it.
Only this asks the CLI, drawing does not."
  (cond (samu--habit-id (samu--fetch-habit))
        (samu--task-id (samu--fetch-task))
        (t (samu--fetch-view))))

(defvar-local samu--stale nil
  "Non-nil when this buffer is out of date and in no window.
It fills anew once a window shows it.")

(defvar-local samu--fetching nil
  "Non-nil while this buffer waits for the CLI to be filled anew.
`again' when it was asked to fill anew once more meanwhile.")

(defun samu--render ()
  "Fill this samu buffer anew: a view, a task's or a habit's buffer.
Emacs does not wait: the buffer keeps what it shows until the CLI
has answered.  Asked again meanwhile, it fills once more afterwards."
  (setq samu--stale nil)
  (if samu--fetching
      (setq samu--fetching 'again)
    (let ((buffer (current-buffer)))
      (setq samu--fetching t
            mode-line-process " …")
      (when (and (zerop (buffer-size)) (not samu--sync))
        (samu--placeholder "Loading …"))
      (samu--fetch-async
       (lambda ()
         (if (buffer-live-p buffer)
             (with-current-buffer buffer (samu--fetch))
           (signal 'samu-error (list "samu: the buffer is gone"))))
       (lambda (data) (samu--fetched buffer data nil))
       (lambda (err) (samu--fetched buffer nil err))))))

(defvar-local samu--unfilled nil
  "Non-nil while this buffer shows a placeholder and was never filled.")

(defun samu--placeholder (text)
  "Show TEXT in this buffer in place of what it has yet to be filled with."
  (let ((inhibit-read-only t))
    (erase-buffer)
    (insert (propertize text 'face 'samu-meta) "\n")
    (setq samu--unfilled t)))

(defun samu--fetched (buffer data err)
  "The CLI answered for BUFFER: draw DATA, or ERR came instead.
A buffer filled before keeps what it shows on an error; one never
filled shows the error.  Then fill again if asked to meanwhile."
  (when (buffer-live-p buffer)
    (with-current-buffer buffer
      (let ((again (eq samu--fetching 'again)))
        (setq samu--fetching nil
              mode-line-process nil)
        (unwind-protect
            (cond ((null err)
                   (setq samu--unfilled nil)
                   (samu--draw data))
                  (samu--unfilled
                   (samu--placeholder (concat (samu--error-text err)
                                              "\n\ng asks again"))))
          (force-mode-line-update)
          (when again (samu--render)))))))

(defun samu--draw (data)
  "Fill this samu buffer from DATA, what `samu--fetch' returned.
Point stays on its task, and each window showing the buffer keeps
where it starts, also when the buffer is filled from elsewhere."
  (let* ((windows (get-buffer-window-list nil nil t))
         (starts (mapcar (lambda (w)
                           (let ((start (window-start w)))
                             (list w start (samu--id-from start))))
                         windows)))
    (when (and windows (not (eq (window-buffer) (current-buffer))))
      (goto-char (window-point (car windows))))
    (cond (samu--habit-id (samu--draw-habit data))
          (samu--task-id (samu--draw-task data))
          (t (samu--draw-view data)))
    (pcase-dolist (`(,window ,start ,id) starts)
      (set-window-start window (or (samu--line-of id) (min start (point-max))) t)
      (set-window-point window (point)))))

(defun samu--id-from (pos)
  "The id of the first task at or after POS, or nil."
  (save-excursion
    (goto-char pos)
    (let ((match (text-property-search-forward 'samu-id nil nil)))
      (and match (prop-match-value match)))))

(defun samu--line-of (id)
  "Where the line of the task ID starts, or nil."
  (when id
    (save-excursion
      (goto-char (point-min))
      (let ((match (text-property-search-forward 'samu-id id t)))
        (and match (prop-match-beginning match))))))

(defun samu--goto (id &optional line)
  "Put point on the task ID, else on LINE, else on the first task."
  (goto-char (point-min))
  (let ((match (and id (text-property-search-forward 'samu-id id t))))
    (cond (match (goto-char (prop-match-beginning match)))
          (line (forward-line (1- line))
                (unless (get-text-property (point) 'samu-id)
                  (samu-next-task 1)))
          (t (samu-next-task 1)))))

(defun samu--refresh-all ()
  "Fill the samu buffers anew that are to be seen.
That is this buffer and those in a window, on any frame.  The others
are only noted as out of date: `samu--shown' fills them once a window
shows them."
  (let ((here (current-buffer)))
    (dolist (buffer (samu--live-buffers))
      (with-current-buffer buffer
        (if (or (eq buffer here) (get-buffer-window buffer t))
            (condition-case err
                (samu--render)
              (samu-error (message "%s" (samu--error-text err))))
          (setq samu--stale t))))))

(defun samu--shown (window)
  "WINDOW now shows this buffer: fill it anew if it is out of date."
  (when (window-live-p window)
    (with-current-buffer (window-buffer window)
      (when samu--stale (samu--render)))))

;;;; The list mode

(defvar samu-list-mode-map (make-sparse-keymap)
  "Keys of the samu views.")

(defvar samu-habit-map (make-sparse-keymap)
  "Keys after `h' in a samu buffer: entering a habit (ADR 0091 §6).")

;; Bound at top level, not in the `defvar': loading the file again
;; then takes new keys, also into buffers already open.
(let ((map samu-habit-map))
  (define-key map (kbd "h") #'samu-habit-done)
  (define-key map (kbd "d") #'samu-habit-done)
  (define-key map (kbd "f") #'samu-habit-fail)
  (define-key map (kbd "p") #'samu-habit-pause)
  (define-key map (kbd "u") #'samu-habit-undo)
  (define-key map (kbd "s") #'samu-habit-show))

;; Bound at top level, not in the `defvar': loading the file again
;; then takes new keys, also into buffers already open.
(let ((map samu-list-mode-map))
  ;; Set here, not left to `define-derived-mode': `samu-task-mode-map'
  ;; inherits from this map, and `?' and `q' must reach it too.
  (set-keymap-parent map special-mode-map)
  (define-key map (kbd "?") #'describe-mode)
  (define-key map (kbd "q") #'quit-window)
  (define-key map (kbd "n") #'samu-next-task)
  (define-key map (kbd "p") #'samu-previous-task)
  (define-key map (kbd "j") #'samu-next-task)
  (define-key map (kbd "k") #'samu-previous-task)
  (define-key map (kbd "RET") #'samu-show-at-point)
  (define-key map (kbd "TAB") #'samu-show-other-window)
  (define-key map (kbd "x") #'samu-toggle-done)
  (define-key map (kbd "s") #'samu-next-state)
  (define-key map (kbd "t") #'samu-pick-state)
  (define-key map (kbd "f") #'samu-toggle-flag)
  (define-key map (kbd "c") #'samu-clock-toggle)
  (define-key map (kbd "I") #'samu-clock-in)
  (define-key map (kbd "O") #'samu-clock-out)
  (define-key map (kbd "m") #'samu-move)
  (define-key map (kbd "C-c C-w") #'samu-move)
  (define-key map (kbd "e") #'samu-edit)
  (define-key map (kbd ",") #'samu-set-priority)
  (define-key map (kbd ":") #'samu-set-tags)
  (define-key map (kbd "C-c C-s") #'samu-set-planned)
  (define-key map (kbd "C-c C-d") #'samu-set-due)
  (define-key map (kbd "a") #'samu-capture)
  (define-key map (kbd "A") #'samu-capture-below)
  (define-key map (kbd "C") #'samu-capture-template)
  (define-key map (kbd "/") #'samu-search)
  (define-key map (kbd "X") #'samu-drop)
  (define-key map (kbd "DEL") #'samu-delete)
  (define-key map (kbd "<deletechar>") #'samu-delete)
  (define-key map (kbd "<delete>") #'samu-delete)
  (define-key map (kbd "w") #'samu-copy-link)
  (define-key map (kbd "M") #'samu-mark)
  (define-key map (kbd "u") #'samu-unmark)
  (define-key map (kbd "U") #'samu-unmark-all)
  (define-key map (kbd "B") #'samu-bulk)
  (define-key map (kbd "S") #'samu-choose-scope)
  (define-key map (kbd "R") #'samu-mark-reviewed)
  (define-key map (kbd "h") samu-habit-map)
  (define-key map (kbd "g") #'samu-refresh))

(define-derived-mode samu-list-mode special-mode "samu"
  "A view of samudo's tasks.

\\{samu-list-mode-map}"
  (setq truncate-lines t)
  (setq-local revert-buffer-function (lambda (&rest _) (samu-refresh)))
  (add-hook 'window-buffer-change-functions #'samu--shown nil t)
  (add-hook 'kill-buffer-hook #'samu--live-after-kill nil t))

(with-eval-after-load 'evil
  (evil-set-initial-state 'samu-list-mode 'emacs)
  (evil-set-initial-state 'samu-task-mode 'emacs))

(defun samu-next-task (n)
  "Move to the next task, N times."
  (interactive "p")
  (let ((step (if (< n 0) -1 1)))
    (dotimes (_ (abs n))
      (let ((start (point)))
        (forward-line step)
        (while (and (not (get-text-property (point) 'samu-id))
                    (if (> step 0) (not (eobp)) (not (bobp))))
          (forward-line step))
        (unless (get-text-property (point) 'samu-id)
          (goto-char start))))))

(defun samu-previous-task (n)
  "Move to the previous task, N times."
  (interactive "p")
  (samu-next-task (- n)))

(defun samu-refresh ()
  "Ask the server again; forget what completion offers."
  (interactive)
  (samu--forget)
  (samu--render)
  (samu--clock-update))

(defun samu--open-view (name title fetch &optional project)
  "Show the view NAME with TITLE, filled by FETCH; PROJECT shows projects."
  (let ((buffer (get-buffer-create (format "*samu: %s*" name))))
    (with-current-buffer buffer
      (samu-list-mode)
      (setq samu--view (list :title title :fetch fetch :project project))
      (samu--render))
    (pop-to-buffer-same-window buffer)
    (samu--live-ensure)
    buffer))

;;;; Views

(defun samu--list (&rest args)
  "Rows of `samu list' with ARGS, through the scope."
  (samu--call (append '("list") args (samu--scope-args))))

(defun samu--group (rows key)
  "ROWS in groups by the string KEY returns for each, in the order they came."
  (let ((groups nil))
    (dolist (row rows)
      (let* ((name (funcall key row))
             (cell (assoc name groups)))
        (if cell (push row (cdr cell)) (push (list name row) groups))))
    (nreverse (mapcar (lambda (g) (cons (car g) (nreverse (cdr g)))) groups))))

(defun samu--by-project (rows)
  "ROWS grouped by project, the inbox first."
  (let* ((groups (samu--group rows (lambda (row)
                                     (let ((p (samu--text (samu--get row 'project))))
                                       (if (string-empty-p p) "Inbox" p)))))
         (inbox (assoc "Inbox" groups)))
    (if inbox (cons inbox (delq inbox groups)) groups)))

(defun samu--section-title (section)
  "The heading for the forecast SECTION: a day as weekday and date."
  (if (string-match "\\`\\([0-9]\\{4\\}\\)-\\([0-9]\\{2\\}\\)-\\([0-9]\\{2\\}\\)\\'" section)
      (format-time-string
       "%A %Y-%m-%d"
       (encode-time (list 0 0 12
                          (string-to-number (match-string 3 section))
                          (string-to-number (match-string 2 section))
                          (string-to-number (match-string 1 section))
                          nil -1 nil)))
    (capitalize section)))

(defun samu--forecast-row (row open)
  "The forecast ROW joined with its row of `samu list' from OPEN, a hash table."
  (let ((why (samu--get row 'why)))
    (if (equal why "termin")
        `((title . ,(format "%s %s" (samu--get row 'date) (samu--get row 'title)))
          (why . "appointment"))
      (let ((full (gethash (samu--get row 'id) open)))
        (if full
            (cons (cons 'why why) (assq-delete-all 'why (copy-alist full)))
          row)))))

(defun samu--forecast-groups ()
  "The forecast in sections, each row joined with its row of `samu list'."
  (let ((rows (samu--call (append '("forecast") (samu--scope-args))))
        (open (make-hash-table :test #'equal)))
    (dolist (row (samu--list)) (puthash (samu--get row 'id) row open))
    (mapcar (lambda (group)
              (cons (samu--section-title (car group))
                    (mapcar (lambda (row) (samu--forecast-row row open)) (cdr group))))
            (samu--group rows (lambda (row) (samu--text (samu--get row 'section)))))))

;;;###autoload
(defun samu ()
  "The forecast: overdue, today, flagged, the next days, later."
  (interactive)
  (samu-forecast))

;;;###autoload
(defun samu-forecast ()
  "The forecast as the server computes it."
  (interactive)
  (samu--open-view "forecast" "Forecast" #'samu--forecast-groups t))

(defun samu--inbox-rows ()
  "The open tasks without a project."
  (cl-remove-if-not (lambda (row) (string-empty-p (samu--text (samu--get row 'project))))
                    (samu--list)))

;;;###autoload
(defun samu-inbox ()
  "The tasks without a project."
  (interactive)
  (samu--open-view "inbox" "Inbox" (lambda () (list (cons nil (samu--inbox-rows))))))

;;;###autoload
(defun samu-available ()
  "The available tasks, by project."
  (interactive)
  (samu--open-view "available" "Available"
                   (lambda () (samu--by-project (samu--list "--available")))))

;;;###autoload
(defun samu-today ()
  "The tasks planned for today or earlier, by project."
  (interactive)
  (samu--open-view "today" "Today"
                   (lambda () (samu--by-project (samu--list "--today")))))

;;;###autoload
(defun samu-flagged ()
  "The flagged tasks, by project."
  (interactive)
  (samu--open-view "flagged" "Flagged"
                   (lambda () (samu--by-project (samu--list "--flagged")))))

;;;###autoload
(defun samu-project (id)
  "The open tasks of the project ID."
  (interactive (list (samu--read-project "Project: ")))
  (let ((title (concat "#" (or (samu--project-title id) id))))
    (samu--open-view title title (lambda () (list (cons nil (samu--list "--project" id)))))))

;;;###autoload
(defun samu-tag (path)
  "The open tasks with the tag PATH or one below it, by project."
  (interactive (list (completing-read "Tag: " (mapcar (lambda (tag) (samu--get tag 'path))
                                                    (samu--tags))
                                      nil t)))
  (samu--open-view (concat "@" path) (concat "@" path)
                   (lambda () (samu--by-project (samu--list "--tag" path)))))

;;;###autoload
(defun samu-search (words)
  "The open tasks with every one of WORDS in title, note or id."
  (interactive (list (read-string "Search: ")))
  (samu--open-view (concat "/" words) (concat "Search: " words)
                   (lambda () (samu--by-project (samu--list "--search" words)))))

(defun samu--project-rows (&optional folder review)
  "The projects as rows, only those in FOLDER when given.
With REVIEW, only those due for review, through the scope (ADR 0088 §8)."
  (cl-loop for p in (samu--call (if review
                                    (append '("projects" "--review") (samu--scope-args))
                                  '("projects" "--no-scope")))
           for project = (samu--get p 'project 'project)
           when (or (null folder) (equal (samu--get project 'folder) folder))
           collect `((id . ,(samu--get p 'id))
                     (kind . "project")
                     (title . ,(samu--get p 'title))
                     (available . ,(samu--get p 'available))
                     (state . ,(pcase (samu--get p 'status) ("active" nil) (s s)))
                     (flag . ,(if (samu--get project 'flagged) "!" ""))
                     (why . ,(string-join
                              (cl-remove-if #'string-empty-p
                                            (list (if (string-empty-p (samu--text (samu--get p 'stuck)))
                                                      "" "stuck")
                                                  (if (string-empty-p (samu--text (samu--get p 'review_due)))
                                                      ""
                                                    (concat "review due since "
                                                            (samu--text (samu--get p 'review))))
                                                  (samu--text (samu--get p 'reasons))))
                              ", ")))))

;;;###autoload
(defun samu-projects (&optional folder)
  "Every project with its availability; RET opens one.
With FOLDER, an id, only the projects in that folder."
  (interactive)
  (samu--open-view (if folder (concat "folder " folder) "projects") "Projects"
                   (lambda () (list (cons nil (samu--project-rows folder))))))

;;;###autoload
(defun samu-review ()
  "The projects due for review, oldest first; `R' marks one reviewed."
  (interactive)
  (samu--open-view "review" "Review"
                   (lambda () (list (cons nil (samu--project-rows nil t))))))

(defun samu-mark-reviewed ()
  "Mark the project here reviewed: the project row, or the task's project.
In a project's own buffer, that project (ADR 0088 §3, §8)."
  (interactive)
  (let* ((row (samu--row-at-point))
         (id (cond ((equal (samu--get row 'kind) "project") (samu--get row 'id))
                   (row (samu--get row 'status 'task 'project))
                   (samu--task-object (and (null samu--task-row) samu--task-id)))))
    (unless (and id (not (string-empty-p (samu--text id))))
      (user-error "No project here"))
    (samu--call-text (list "review" id))
    (samu--after-action "Marked reviewed")))

;;;; Acting on tasks

(defun samu--id-at-point ()
  "The id of the task on this line, or of the buffer's task or habit."
  (or (get-text-property (point) 'samu-id) samu--task-id samu--habit-id))

(defun samu--row-at-point ()
  "The row on this line, or the buffer's task's row."
  (or (get-text-property (point) 'samu-row) samu--task-row))

(defun samu--title-at-point ()
  "The title of the task here, or of the buffer's task or project."
  (samu--text (if (get-text-property (point) 'samu-row)
                  (samu--get (get-text-property (point) 'samu-row) 'title)
                (samu--get samu--task-object 'title))))

(defun samu--targets ()
  "The ids to act on: the marked ones, else the task here."
  (or (reverse samu--marked)
      (let ((id (samu--id-at-point)))
        (if id (list id) (user-error "No task here")))))

(defun samu--target-rows ()
  "The rows of the ids to act on, where the buffer has them."
  (let ((ids (samu--targets)))
    (or (cl-remove-if-not (lambda (row) (member (samu--get row 'id) ids))
                          (append samu--rows (and samu--task-row (list samu--task-row))))
        (mapcar (lambda (id) `((id . ,id))) ids))))

(defun samu--after-action (text)
  "Show TEXT, unmark and fill the samu buffers anew."
  (setq samu--marked nil)
  (samu--refresh-all)
  (when text (message "%s" text)))

(defun samu--bulk (command ids &rest args)
  "Run `samu bulk COMMAND' with ARGS on IDS."
  (samu--call (append (list "bulk" command) args ids)))

(defun samu--count (ids)
  "\"1 task\" or \"N tasks\" for IDS."
  (if (cdr ids) (format "%d tasks" (length ids)) "1 task"))

(defun samu--choice-question (err ids)
  "The question for IDS from the stderr ERR of exit code 7.
ERR lists each waiting task, the open tasks below it indented."
  (let* ((lines (split-string err "\n" t))
         (tasks (cl-count-if (lambda (l) (string-match-p "\\`[[:alnum:]]" l))
                             (cl-remove-if (lambda (l) (string-prefix-p "samu:" l))
                                           lines)))
         (below (cl-count-if (lambda (l) (string-prefix-p " " l)) lines)))
    (if (cdr ids)
        (format "%d of the %d tasks have open subtasks. What happens to them? "
                (max tasks 1) (length ids))
      (if (= below 1)
          "1 open subtask. What happens to it? "
        (format "%d open subtasks. What happens to them? " below)))))

(defun samu--close (command ids along)
  "Run `samu bulk COMMAND' on IDS; ask when tasks have open tasks below.
ALONG names the first answer, \"complete them too\" or \"drop them too\".
Return non-nil when something was closed (ADR 0128 §5)."
  (pcase-let ((`(,code ,_out ,err)
               (samu--run (append (list "bulk" command) ids (list "--format" "json")))))
    (cond
     ((eql code 0) t)
     ((eql code samu--exit-needs-choice)
      (let ((answer (car (read-multiple-choice
                          (samu--choice-question err ids)
                          `((?t ,along "the open subtasks as well")
                            (?l "lift out" "one level up, behind the task")
                            (?c "cancel" "change nothing"))))))
        (pcase answer
          (?t (samu--bulk command ids "--children" "close") t)
          (?l (samu--bulk command ids "--children" "lift") t)
          (_ (message "Nothing changed") nil))))
     (t (samu--fail code err)))))

(defun samu-toggle-done ()
  "Complete the tasks, or open them again if all of them are closed.
With open tasks below, ask whether they are completed along or lifted out."
  (interactive)
  (let* ((ids (samu--targets))
         (rows (samu--target-rows))
         (reopen (cl-every #'samu--row-closed-p rows)))
    (if reopen
        (progn (samu--bulk "reopen" ids)
               (samu--after-action (format "%s reopened" (samu--count ids))))
      (when (samu--close "done" ids "complete them too")
        (samu--after-action (format "%s completed" (samu--count ids)))))))

(defun samu-drop ()
  "Drop the tasks, after asking.
With open tasks below, ask whether they are dropped along or lifted out."
  (interactive)
  (let ((ids (samu--targets)))
    (when (and (yes-or-no-p (format "Drop %s? " (samu--count ids)))
               (samu--close "drop" ids "drop them too"))
      (samu--after-action (format "%s dropped" (samu--count ids))))))

(defun samu-delete ()
  "Delete the tasks with their children, after asking (ADR 0036)."
  (interactive)
  (let ((ids (samu--targets)))
    (when (yes-or-no-p (format "Delete %s with subtasks? " (samu--count ids)))
      (samu--bulk "delete" ids)
      (samu--after-action (format "%s deleted" (samu--count ids))))))

(defun samu-toggle-flag ()
  "Flag the tasks, or take the flag off if all of them have it."
  (interactive)
  (let* ((ids (samu--targets))
         (unflag (cl-every (lambda (row) (samu--get (samu--row-task row) 'flagged))
                           (samu--target-rows))))
    (samu--bulk (if unflag "unflag" "flag") ids)
    (samu--after-action nil)))

(defun samu-next-state ()
  "Put the tasks into the next state of their workflow."
  (interactive)
  (samu--bulk "state" (samu--targets) "--next")
  (samu--after-action nil))

(defun samu--workflow-states (workflow)
  "The states of the WORKFLOW, a tag id, as (KEY TITLE) lists."
  (let ((path (samu--tag-path workflow)))
    (when path
      (cl-loop for tag in (samu--tags)
               for p = (samu--get tag 'path)
               when (and (string-prefix-p (concat path "/") p)
                         (not (string-search "/" (substring p (1+ (length path))))))
               collect (list (samu--text (samu--get tag 'key))
                             (substring p (1+ (length path))))))))

(defun samu-pick-state ()
  "Pick a state of the workflow by its key, like `t' in the web UI."
  (interactive)
  (let* ((workflow (samu--get (car (samu--target-rows)) 'status 'workflow))
         (states (cl-remove-if (lambda (s) (string-empty-p (car s)))
                               (samu--workflow-states workflow))))
    (unless states (user-error "No workflow with keys"))
    (let ((char (read-char-choice
                 (concat (mapconcat (lambda (s) (format "[%s] %s" (car s) (cadr s)))
                                    states " ")
                         " [SPC] none: ")
                 (cons ?\s (mapcar (lambda (s) (string-to-char (car s))) states)))))
      (if (eq char ?\s)
          (samu--bulk "state" (samu--targets) "--none")
        (samu--bulk "state" (samu--targets) "--to" (char-to-string char)))
      (samu--after-action nil))))

(defun samu-set-priority ()
  "Set the priority: A, B, C, or none with SPC."
  (interactive)
  (let ((char (read-char-choice "Priority [A] [B] [C] [SPC] none: "
                                '(?a ?b ?c ?A ?B ?C ?\s))))
    (dolist (id (samu--targets))
      (if (eq char ?\s)
          (samu--call (list "unset" id "priority"))
        (samu--call (list "set" id "priority" (upcase (char-to-string char))))))
    (samu--after-action nil)))

(defun samu--current-tags (rows)
  "The paths of the plain tags all of ROWS carry."
  (let ((sets (mapcar (lambda (row)
                        (delq nil (mapcar (lambda (tagging)
                                            (samu--tag-path (samu--get tagging 'tag)))
                                          (samu--get (samu--row-task row) 'tags))))
                      rows))
        (plain (samu--plain-tag-paths)))
    (cl-remove-if-not (lambda (path)
                        (and (member path plain)
                             (cl-every (lambda (set) (member path set)) sets)))
                      (car sets))))

(defun samu--tag-changes (before after)
  "The arguments of `samu bulk tag' to go from the tags BEFORE to AFTER."
  (append
   (cl-mapcan (lambda (path) (list "--add" path))
              (cl-remove-if (lambda (p) (member p before)) after))
   (cl-mapcan (lambda (path) (list "--remove" path))
              (cl-remove-if (lambda (p) (member p after)) before))))

(defun samu-set-tags ()
  "Set the tags: the ones read are on afterwards, the others off.
States of a workflow stay as they are; `t' sets them."
  (interactive)
  (let* ((before (samu--current-tags (samu--target-rows)))
         (after (completing-read-multiple
                 "Tags: " (samu--plain-tag-paths) nil nil
                 (and before (concat (string-join before ",") ","))))
         (args (samu--tag-changes before (delete-dups (cl-remove-if #'string-empty-p after)))))
    (when args
      (let ((known (mapcar (lambda (tag) (samu--get tag 'path)) (samu--tags))))
        ;; a new tag, and the ones above it first: `tag add a/b' needs a
        (dolist (path after)
          (let ((parts (split-string path "/" t)) (prefix nil))
            (dolist (part parts)
              (setq prefix (if prefix (concat prefix "/" part) part))
              (unless (member prefix known)
                (samu--call (list "tag" "add" prefix))
                (push prefix known))))))
      (apply #'samu--bulk "tag" (samu--targets) args)
      (samu--forget)
      (samu--after-action nil))))

(defun samu--read-date (prompt clear)
  "A date read with `org-read-date' and PROMPT, or \"none\" if CLEAR."
  (if clear
      "none"
    (require 'org)
    (org-read-date nil nil nil prompt)))

(defun samu-set-planned (clear)
  "Set the planned date; with C-u (CLEAR) take it off."
  (interactive "P")
  (samu--bulk "date" (samu--targets) "planned" (samu--read-date "Planned: " clear))
  (samu--after-action nil))

(defun samu-set-due (clear)
  "Set the due date; with C-u (CLEAR) take it off."
  (interactive "P")
  (samu--bulk "date" (samu--targets) "due" (samu--read-date "Due: " clear))
  (samu--after-action nil))

(defun samu--open-tasks ()
  "Every open task, through no scope, as rows."
  (samu--call '("list" "--no-scope")))

(defun samu--read-task (prompt)
  "Read an open task with PROMPT; return its id."
  (let ((table (mapcar (lambda (row)
                         (cons (format "%s  %s" (samu--get row 'title) (samu--get row 'short))
                               (samu--get row 'id)))
                       (samu--open-tasks))))
    (cdr (assoc (completing-read prompt table nil t) table))))

(defun samu-move (group)
  "Move the tasks into the inbox or a project; with C-u (GROUP) under a task."
  (interactive "P")
  (let ((ids (samu--targets)))
    (if group
        (samu--bulk "move" ids "--group" (samu--read-task "Under the task: "))
      (samu--bulk "move" ids "--to" (samu--read-project "To: " t)))
    (samu--after-action (format "%s moved" (samu--count ids)))))

(defun samu--report (record)
  "Show the warnings and new tags of the capture RECORD, if any."
  (let ((parts (append
                (mapcar #'samu--text (samu--get record 'warnings))
                (and (samu--get record 'new_tags)
                     (list (concat "New tag: "
                                   (mapconcat #'samu--text (samu--get record 'new_tags) ", ")))))))
    (when parts (message "%s" (string-join parts " · ")))))

(defun samu-edit (line)
  "Edit the tasks by LINE, marks of quick capture (ADR 0078).
`@tag' adds, `#project !A d:fr ~30m every:1w !' overwrite, `// text'
is appended to the note, `/name' applies a template, `z:' books time."
  (interactive (list (samu--read-line "Edit: ")))
  (let ((records nil))
    (dolist (id (samu--targets))
      (setq records (append records (samu--call (list "edit" id line)))))
    (samu--forget)
    (samu--after-action nil)
    (mapc #'samu--report records)))

(defun samu-copy-link ()
  "Put the `samu:' link of the task here into the kill ring."
  (interactive)
  (let ((id (or (samu--id-at-point) (user-error "No task here"))))
    (kill-new (concat "samu:" id))
    (message "samu:%s" id)))

;;;; Marking

(defun samu--set-mark-here (on)
  "Draw the mark of this line: ON or off."
  (let ((inhibit-read-only t)
        (props (text-properties-at (line-beginning-position))))
    (save-excursion
      (goto-char (line-beginning-position))
      (when (looking-at "> \\|  ")
        (replace-match (if on "> " "  ") t t)
        (set-text-properties (line-beginning-position) (+ 2 (line-beginning-position)) props)
        (when on
          (put-text-property (line-beginning-position) (+ 2 (line-beginning-position))
                             'face 'samu-marked))))))

(defun samu-mark ()
  "Mark the task here and go on to the next."
  (interactive)
  (let ((id (or (get-text-property (point) 'samu-id) (user-error "No task here"))))
    (cl-pushnew id samu--marked :test #'equal)
    (samu--set-mark-here t)
    (samu-next-task 1)))

(defun samu-unmark ()
  "Unmark the task here and go on to the next."
  (interactive)
  (let ((id (get-text-property (point) 'samu-id)))
    (when id
      (setq samu--marked (delete id samu--marked))
      (samu--set-mark-here nil))
    (samu-next-task 1)))

(defun samu-unmark-all ()
  "Unmark every task."
  (interactive)
  (setq samu--marked nil)
  (samu--render))

(defun samu-bulk ()
  "Pick an action for the marked tasks (ADR 0052)."
  (interactive)
  (unless samu--marked (user-error "Nothing marked"))
  (let ((choice (read-multiple-choice
                 (samu--count samu--marked)
                 '((?x "complete") (?X "drop") (?f "flag")
                   (?m "move") (?t "state") (?s "next state")
                   (?, "priority") (?: "tags") (?p "planned") (?d "due")
                   (?e "edit") (?D "delete")))))
    (call-interactively
     (pcase (car choice)
       (?x #'samu-toggle-done) (?X #'samu-drop) (?f #'samu-toggle-flag)
       (?m #'samu-move) (?t #'samu-pick-state) (?s #'samu-next-state)
       (?, #'samu-set-priority) (?: #'samu-set-tags) (?p #'samu-set-planned)
       (?d #'samu-set-due) (?e #'samu-edit) (?D #'samu-delete)))))

;;;; Scope and account

(defun samu-choose-scope ()
  "Choose the scope the views look through, for this Emacs."
  (interactive)
  (let* ((titles (mapcar (lambda (s) (samu--get s 'title)) (samu--call '("scope" "list"))))
         (choice (completing-read "Scope: " (append '("(as the CLI)" "(no scope)") titles)
                                  nil t)))
    (setq samu-scope (pcase choice
                       ("(as the CLI)" nil)
                       ("(no scope)" 'none)
                       (title title)))
    (samu--refresh-all)))

(defun samu--switched ()
  "After another account or profile: forget, follow it, show it."
  (samu--forget)
  (when (samu--live-running-p)
    (samu--live-stop)
    (samu--live-start))
  (samu--refresh-all)
  (samu--clock-update))

(defun samu-switch-account (account)
  "Act on ACCOUNT from now on, in this Emacs; empty for the CLI's."
  (interactive (list (read-string "Account (empty: as the CLI): " nil nil samu-account)))
  (setq samu-account (if (string-empty-p account) nil account))
  (samu--switched))

(defun samu--profile-names ()
  "The profiles of this computer, from `samu profile list'."
  (mapcar (lambda (p) (samu--get p 'name))
          (let ((samu-profile nil) (samu-server nil) (samu-account nil))
            (samu--call '("profile" "list")))))

(defun samu-switch-profile (profile)
  "Act on PROFILE from now on, in this Emacs; empty for the CLI's default.
A profile names server and account at once (ADR 0086), so both
`samu-server' and `samu-account' are cleared."
  (interactive
   (list (completing-read "Profile (empty: as the CLI): "
                          (samu--profile-names) nil t nil nil samu-profile)))
  (setq samu-profile (if (string-empty-p profile) nil profile)
        samu-server nil
        samu-account nil)
  (samu--switched))

;;;; Quick capture

(defun samu--capture-candidates (prefix)
  "What completes a word starting with the character PREFIX: #, @ or /."
  (pcase prefix
    (?# (mapcar (lambda (p) (concat "#" (samu--get p 'title))) (samu--projects)))
    (?@ (mapcar (lambda (path) (concat "@" path)) (samu--plain-tag-paths)))
    (?/ (mapcar (lambda (tpl) (or (samu--get tpl 'call) (concat "/" (samu--get tpl 'title))))
                (samu--templates)))))

(defun samu--capture-capf ()
  "Complete #project, @tag and /template in a line of quick capture."
  (let ((end (point))
        (start (save-excursion (skip-chars-backward "^ \t\n") (point))))
    (when (and (< start end)
               (memq (char-after start) '(?# ?@ ?/))
               (not (string-prefix-p "//" (buffer-substring-no-properties start end))))
      (let ((prefix (char-after start)))
        (list start end
              (completion-table-dynamic (lambda (_) (samu--capture-candidates prefix)))
              :exclusive 'no)))))

(defvar samu-capture-minibuffer-map (make-sparse-keymap)
  "Keys while reading a line of quick capture.")

;; Bound at top level, not in the `defvar': loading the file again
;; then takes new keys, also into buffers already open.
(let ((map samu-capture-minibuffer-map))
  (set-keymap-parent map minibuffer-local-map)
  (define-key map (kbd "TAB") #'completion-at-point))

(defun samu--read-line (prompt &optional initial)
  "Read a line of quick capture with PROMPT and INITIAL text."
  (minibuffer-with-setup-hook
      (lambda () (add-hook 'completion-at-point-functions #'samu--capture-capf nil t))
    (read-from-minibuffer prompt initial samu-capture-minibuffer-map)))

(defun samu--capture-note ()
  "What a capture from here puts into the note: region and where."
  (let ((region (and (use-region-p)
                     (string-trim (buffer-substring-no-properties
                                   (region-beginning) (region-end)))))
        (link (and samu-capture-link buffer-file-name
                   (format "[[file:%s::%d][%s]]"
                           (abbreviate-file-name buffer-file-name)
                           (line-number-at-pos)
                           (file-name-nondirectory buffer-file-name)))))
    (string-join (delq nil (list region link)) "\n\n")))

(defun samu--capture-text (line note)
  "The text for `samu add': LINE with NOTE behind it, if any."
  (cond ((string-empty-p note) line)
        ((string-search "//" line) (concat line "\n\n" note))
        (t (concat line " // " note))))

;;;###autoload
(defun samu-capture (&optional parent initial)
  "Capture a task by a line of quick capture (ADR 0011).
`#project @tag ! !A d:fr s:mo p:tomorrow ~15m every:1w /template // note'.
An active region and, in a file, where it came from go into the note.
PARENT, an id, puts the task below that task.  INITIAL is the line to
start from, as `read-from-minibuffer' takes it.  With \\[universal-argument] \\[universal-argument],
choose a template by its key first, like `samu-capture-template'."
  (interactive (and (equal current-prefix-arg '(16))
                    (list nil (samu--template-line (samu--choose-template)))))
  (let* ((note (samu--capture-note))
         (line (string-trim (samu--read-line (if parent "Subtask: " "Capture: ")
                                             initial))))
    (when (string-empty-p line) (user-error "Nothing to capture"))
    ;; Several lines pasted in make a task each (ADR 0129).
    (if (samu--several-p line)
        (samu-capture-lines line parent)
      (let ((record (car (samu--call (append '("add") (and parent (list "--parent" parent))
                                             (list (samu--capture-text line note)))))))
        (deactivate-mark)
        (samu--forget)
        (samu--refresh-all)
        (message "Captured: %s  %s" (samu--get record 'title) (samu--get record 'short))
        (samu--report record)
        record))))

(defun samu--several-p (text)
  "Whether TEXT is several lines to capture: two that are not blank."
  (>= (cl-count-if (lambda (line) (not (string-blank-p line)))
                   (split-string text "\n"))
      2))

;;;###autoload
(defun samu-capture-lines (&optional text parent)
  "Capture a task per line of TEXT, all or none (ADR 0129).
Each line is a line of quick capture; list markers go, and a line
indented deeper than the one before goes under it.  Interactively, TEXT
is the region; without one, a buffer opens to write the lines in.
PARENT, an id, puts the top lines below that task."
  (interactive
   (list (and (use-region-p)
              (buffer-substring-no-properties (region-beginning) (region-end)))))
  (if text
      (samu--send-lines text parent)
    (samu--lines-buffer parent)))

(defun samu--send-lines (text parent)
  "Send TEXT to `samu add --lines', below PARENT if any; return the records."
  (let ((records (samu--call (append '("add" "--lines")
                                     (and parent (list "--parent" parent))
                                     '("-"))
                             text)))
    (deactivate-mark)
    (samu--forget)
    (samu--refresh-all)
    (message "Captured: %d tasks" (length records))
    records))

(defvar-local samu--lines-parent nil
  "The task the lines of this buffer go below, or nil.")

(defvar samu-lines-mode-map
  (let ((map (make-sparse-keymap)))
    (define-key map (kbd "C-c C-c") #'samu-lines-finish)
    (define-key map (kbd "C-c C-k") #'samu-lines-cancel)
    map)
  "Keys of the buffer for several lines.")

(define-derived-mode samu-lines-mode text-mode "samu lines"
  "A task per line, an indented line below the one above it (ADR 0129).
\\{samu-lines-mode-map}"
  (add-hook 'completion-at-point-functions #'samu--capture-capf nil t)
  (setq header-line-format
        (substitute-command-keys
         "One task per line · \\[samu-lines-finish] captures · \\[samu-lines-cancel] discards")))

(defun samu--lines-buffer (parent)
  "Open the buffer for several lines, below PARENT if any."
  (let ((buffer (get-buffer-create "*samu lines*")))
    (with-current-buffer buffer
      (erase-buffer)
      (samu-lines-mode)
      (setq samu--lines-parent parent))
    (pop-to-buffer buffer)
    buffer))

(defun samu-lines-finish ()
  "Capture the lines of this buffer and close it; on an error it stays."
  (interactive)
  (let ((text (buffer-substring-no-properties (point-min) (point-max))))
    (when (string-blank-p text) (user-error "Nothing to capture"))
    (prog1 (samu--send-lines text samu--lines-parent)
      (quit-window t))))

(defun samu-lines-cancel ()
  "Close the buffer for several lines without capturing."
  (interactive)
  (quit-window t))

(defun samu--choose-template ()
  "A template chosen by its key, as `org-capture' does (ADR 0084 §5).
A small window lists the templates with a key; \\[keyboard-quit] gives up."
  (let* ((keyed (cl-remove-if-not
                 (lambda (template) (= 1 (length (samu--text (samu--get template 'key)))))
                 (samu--templates)))
         (keys (mapcar (lambda (template) (string-to-char (samu--get template 'key))) keyed)))
    (unless keyed
      (user-error "No template with a key: samu template key <name> <key>"))
    (save-window-excursion
      (with-current-buffer (get-buffer-create "*samu templates*")
        (let ((inhibit-read-only t))
          (erase-buffer)
          (dolist (template keyed)
            (insert (format "[%s]  %-14s %s\n"
                            (samu--get template 'key)
                            (samu--get template 'call)
                            (samu--text (samu--get template 'text)))))
          (goto-char (point-min))
          (special-mode))
        (fit-window-to-buffer (display-buffer (current-buffer)
                                              '(display-buffer-at-bottom
                                                (window-height . fit-window-to-buffer)))))
      (let ((key (read-char-choice "Template: " keys)))
        (cl-find key keyed :key (lambda (template) (string-to-char (samu--get template 'key))))))))

(defun samu--template-line (template)
  "The line a capture with TEMPLATE starts from: its call, point in front."
  (cons (concat " " (samu--get template 'call)) 1))

;;;###autoload
(defun samu-capture-template ()
  "Capture a task with a template chosen by its key, like `org-capture'.
`C h' in the views, as in the web (ADR 0084 §5); bind it globally to
your liking, e.g. (global-set-key (kbd \"C-c S\") #\='samu-capture-template)."
  (interactive)
  (samu-capture nil (samu--template-line (samu--choose-template))))

(defun samu-capture-below ()
  "Capture a task below the task here."
  (interactive)
  (samu-capture (or (samu--id-at-point) (user-error "No task here"))))

;;;; Clocking

(defvar samu--clock nil
  "The running clock as `samu clock status' gave it, or nil.")

(defvar samu--clock-timer nil
  "The timer that looks at the running clock.")

(defvar samu-clock-mode-line-mode)

(defun samu--clock-update ()
  "Look at the running clock again, quietly, if the mode line shows it.
Emacs does not wait for the answer."
  (when samu-clock-mode-line-mode
    (samu--run-async
     '("clock" "status" "--format" "json") nil
     (lambda (answer)
       (when (eql (car answer) 0)
         (condition-case nil
             (setq samu--clock (car (samu--parse (nth 1 answer))))
           (error nil))
         (force-mode-line-update t))))))

(defun samu--clock-tick ()
  "Every `samu-clock-interval': count on; ask the CLI only without live updates."
  (if (samu--live-running-p)
      (force-mode-line-update t)
    (samu--clock-update)))

(defun samu--clock-text ()
  "The running clock for the mode line, or nil."
  (when samu--clock
    (let* ((start (parse-iso8601-time-string (samu--get samu--clock 'start)))
           (minutes (max 0 (floor (float-time (time-subtract nil start)) 60))))
      (propertize (format " ⏱ %d:%02d %s" (/ minutes 60) (% minutes 60)
                          (truncate-string-to-width (samu--text (samu--get samu--clock 'title))
                                                    30 nil nil "…"))
                  'help-echo "samu: the running clock"))))

(defvar samu--clock-mode-line '(:eval (samu--clock-text))
  "The mode line's part for the running clock.")
(put 'samu--clock-mode-line 'risky-local-variable t)

;;;###autoload
(define-minor-mode samu-clock-mode-line-mode
  "Show samudo's running clock in the mode line."
  :global t
  (when samu--clock-timer (cancel-timer samu--clock-timer))
  (setq samu--clock-timer nil)
  (setq global-mode-string (delq 'samu--clock-mode-line (or global-mode-string (list ""))))
  (when samu-clock-mode-line-mode
    (setq global-mode-string (append global-mode-string '(samu--clock-mode-line)))
    (samu--clock-update)
    (setq samu--clock-timer (run-at-time samu-clock-interval samu-clock-interval
                                         #'samu--clock-tick)))
  (samu--live-ensure))

(defun samu-clock-in ()
  "Start a clock on the task here; a running one ends."
  (interactive)
  (let ((id (or (samu--id-at-point) (user-error "No task here"))))
    (samu--call (list "clock" "in" id))
    (samu--clock-update)
    (samu--after-action "Clock running")))

;;;###autoload
(defun samu-clock-out ()
  "Stop the running clock."
  (interactive)
  (let ((stopped (car (samu--call '("clock" "out") nil t))))
    (samu--clock-update)
    (if stopped
        (samu--after-action (format "Clock stopped: %s %s"
                                    (samu--text (samu--get stopped 'length))
                                    (samu--text (samu--get stopped 'title))))
      (message "No clock running"))))

(defun samu-clock-toggle ()
  "Stop the clock if it runs on the task here, else start one there."
  (interactive)
  (let ((running (car (samu--call '("clock" "status")))))
    (if (and running (equal (samu--get running 'owner) (samu--id-at-point)))
        (samu-clock-out)
      (samu-clock-in))))

;;;###autoload
(defun samu-clock-report (by from)
  "Time clocked from the day FROM on, BY task, project, folder, tag or day."
  (interactive (list (completing-read "By: " '("task" "project" "folder" "tag" "day")
                                      nil t nil nil "task")
                     (progn (require 'org) (org-read-date nil nil nil "From: "))))
  (let ((text (samu--call-text (list "clock" "report" "--by" by "--from" from)))
        (buffer (get-buffer-create "*samu: time*")))
    (with-current-buffer buffer
      (let ((inhibit-read-only t))
        (erase-buffer)
        (insert (propertize (format "Time from %s by %s" from by) 'face 'samu-title)
                "\n\n" text))
      (special-mode)
      (goto-char (point-min)))
    (pop-to-buffer buffer)))

;;;; Following changes as they happen

(defvar samu--live-process nil
  "The process `samu events' while it runs.")

(defvar samu--live-timer nil
  "The timer that refreshes once after a quiet `samu--live-delay'.")

(defvar samu--live-retry-timer nil
  "The timer that connects again after the process ended.")

(defconst samu--live-delay 1
  "Seconds without an event before the buffers are filled anew.")

(defconst samu--live-first-retry 5
  "Seconds before connecting again the first time; doubles up to a minute.")

(defvar samu--live-retry samu--live-first-retry
  "Seconds before the next try to connect again.")

(defun samu--live-buffers ()
  "The samu buffers that follow changes."
  (cl-remove-if-not (lambda (buffer)
                      (with-current-buffer buffer
                        (and (derived-mode-p 'samu-list-mode)
                             (or samu--view samu--task-id samu--habit-id))))
                    (buffer-list)))

(defun samu--live-wanted-p ()
  "Whether something follows changes: a samu buffer or the clock."
  (and samu-live
       (or samu-clock-mode-line-mode (samu--live-buffers))
       t))

(defun samu--live-running-p ()
  "Whether `samu events' runs."
  (process-live-p samu--live-process))

(defun samu--live-command ()
  "The command line of the process."
  (append (list samu-executable) (samu--global-args) '("events" "--format" "jsonl")))

(defun samu--live-start ()
  "Start `samu events'."
  (when samu--live-retry-timer
    (cancel-timer samu--live-retry-timer)
    (setq samu--live-retry-timer nil))
  (setq samu--live-process
        (make-process :name "samu-events"
                      :command (samu--live-command)
                      :connection-type 'pipe
                      :noquery t
                      :coding 'utf-8
                      :stderr (get-buffer-create " *samu-events*")
                      :filter #'samu--live-filter
                      :sentinel #'samu--live-sentinel)))

(defun samu--live-stop ()
  "Stop `samu events' and what waits on it."
  (dolist (timer (list samu--live-timer samu--live-retry-timer))
    (when timer (cancel-timer timer)))
  (setq samu--live-timer nil
        samu--live-retry-timer nil)
  (let ((process samu--live-process))
    (setq samu--live-process nil)
    (when (process-live-p process)
      (delete-process process))))

(defun samu--live-ensure ()
  "Follow changes if something wants it, else stop."
  (cond ((not (samu--live-wanted-p))
         (samu--live-stop))
        ((not (or (samu--live-running-p) samu--live-retry-timer))
         (samu--live-start))))

(defun samu--live-filter (_process output)
  "An event arrived in OUTPUT: refresh after a quiet moment."
  (when (string-search "\n" output)
    (setq samu--live-retry samu--live-first-retry)
    (samu--live-schedule)))

(defun samu--live-schedule ()
  "Refresh once `samu--live-delay' after the last call."
  (when samu--live-timer (cancel-timer samu--live-timer))
  (setq samu--live-timer (run-at-time samu--live-delay nil #'samu--live-refresh)))

(defun samu--live-refresh ()
  "Fill the samu buffers and the clock anew, unless something is being read."
  (setq samu--live-timer nil)
  (if (active-minibuffer-window)
      (samu--live-schedule)
    (samu--refresh-all)
    (samu--clock-update)))

(defun samu--live-sentinel (process _event)
  "PROCESS ended: connect again later, each time waiting longer."
  (when (and (eq process samu--live-process) (not (process-live-p process)))
    (setq samu--live-process nil)
    (when (samu--live-wanted-p)
      (setq samu--live-retry-timer
            (run-at-time samu--live-retry nil
                         (lambda ()
                           (setq samu--live-retry-timer nil)
                           (samu--live-ensure))))
      (setq samu--live-retry (min 60 (* 2 samu--live-retry))))))

(defun samu--live-after-kill ()
  "A samu buffer goes: stop following changes when it was the last."
  (run-at-time 0 nil #'samu--live-ensure))

(defun samu-toggle-live ()
  "Switch `samu-live': follow changes as they happen, or not."
  (interactive)
  (setq samu-live (not samu-live))
  (samu--live-ensure)
  (message "samu: live updates %s" (if samu-live "on" "off")))

;;;; Habits

;; A habit is a task with the field `habit' (ADR 0091).  What it is
;; today, its streak and its history come from `samu habit list' and
;; `samu habit show', computed in samu-core; samu.el only draws them.
;; Each tap is an entry of its own, written by `samu habit done', `fail'
;; or `pause'.

(defun samu--row-habit (row)
  "The field `habit' of the task in ROW, or nil."
  (samu--get (samu--row-task row) 'habit))

(defun samu--habit-statuses (rows scope-args)
  "The records of `samu habit list' with SCOPE-ARGS, by task id.
One call for all ROWS, and none when no open task among them is a
habit.  If the CLI fails, the rows show without habit; the list matters
more."
  (when (cl-some (lambda (row) (and (samu--row-habit row) (not (samu--row-closed-p row))))
                 rows)
    (condition-case err
        (let ((table (make-hash-table :test #'equal)))
          (dolist (record (samu--call (append '("habit" "list") scope-args)))
            (puthash (samu--get record 'id) record table))
          table)
      (samu-error
       ;; once, not in every run of `samu--fetch-async'
       (unless samu--missing (message "%s" (error-message-string err)))
       nil))))

(defun samu--habit-goal (habit)
  "The goal of HABIT in words, as the web says it.
Like \"daily\", \"3× a week\", \"8 glasses a day\", \"avoid\"."
  (let ((per (pcase (samu--get habit 'per)
               ("week" "a week") ("month" "a month") (_ "a day")))
        (times (or (samu--get habit 'times) 1))
        (unit (samu--get habit 'unit)))
    (cond ((equal (samu--get habit 'kind) "avoid") "avoid")
          (unit (format "%d %s %s" times unit per))
          ((and (= times 1) (member (samu--get habit 'per) '(nil "day"))) "daily")
          (t (format "%d× %s" times per)))))

(defun samu--habit-cell (record)
  "What the row of a habit shows, from its RECORD of `samu habit list'.
To build: ✓ once the period's goal is met, else ✗ for a fail today, ‖
for a pause, ○ for not yet; with a unit or a goal above 1 the amount
like 5/8 glasses, and the streak.  To avoid: \"free for n d\"."
  (let* ((status (samu--get record 'status))
         (habit (samu--get status 'task 'habit))
         (day (samu--get status 'day))
         (period (samu--get status 'period))
         (streak (or (samu--get status 'streak) 0))
         (target (or (samu--get period 'target) 1))
         (unit (samu--get habit 'unit))
         (per (samu--get habit 'per)))
    (if (equal (samu--get habit 'kind) "avoid")
        (propertize (format "free for %d d" streak)
                    'face (if (equal day "fail") 'samu-habit-fail 'samu-habit-met))
      (concat
       (cond ((samu--get period 'met) (propertize "✓" 'face 'samu-habit-met))
             ((equal day "fail") (propertize "✗" 'face 'samu-habit-fail))
             ((equal day "pause") (propertize "‖" 'face 'samu-habit-quiet))
             (t (propertize "○" 'face 'samu-habit-quiet)))
       (if (or unit (> target 1) (member per '("week" "month")))
           (propertize
            (concat (format " %d/%d" (or (samu--get period 'amount) 0) target)
                    (if unit (concat " " unit) "")
                    (pcase per ("week" " week") ("month" " month") (_ "")))
            'face (if (samu--get period 'met) 'samu-habit-met 'samu-habit-quiet))
         "")
       (if (> streak 0)
           (propertize (format " %s%d" samu-habit-streak-mark streak) 'face 'samu-habit-quiet)
         "")))))

(defun samu--habit-targets ()
  "The ids to enter for: the marked tasks, else the task here.
Signal if one the buffer knows is no habit."
  (let ((ids (samu--targets)))
    (dolist (row (samu--target-rows))
      (when (and (samu--row-task row) (not (samu--row-habit row)))
        (user-error "Not a habit: %s" (samu--text (samu--get row 'title)))))
    ids))

(defun samu--habit-enter (ids command &rest args)
  "Run `samu habit COMMAND <id> ARGS' for each of IDS; then show the last."
  (let ((record nil))
    (dolist (id ids)
      (setq record (car (samu--call (append (list "habit" command id) args)))))
    (samu--after-action
     (and record (format "%s: %s" (samu--text (samu--get record 'title))
                         (samu--habit-cell record))))
    record))

(defun samu--habit-day-args (arg prompt)
  "`--day D' read with PROMPT when ARG is \\[universal-argument], else none."
  (and (consp arg) (list "--day" (samu--read-date prompt nil))))

(defun samu-habit-done (arg)
  "Enter the habits here as done today: +1.
A number as ARG (\\[universal-argument] 3, M-3) enters that amount; a plain
\\[universal-argument] asks for an earlier day.  Entering never completes
the habit; `x' does."
  (interactive "P")
  (let ((ids (samu--habit-targets)))
    (apply #'samu--habit-enter ids "done"
           (append (samu--habit-day-args arg "Done on: ")
                   (and (integerp arg)
                        (if (> arg 0)
                            (list "--amount" (number-to-string arg))
                          (user-error "An amount is at least 1")))))))

(defun samu-habit-fail (arg)
  "Enter the habits here as not done today; for a habit to avoid, a relapse.
With \\[universal-argument] (ARG), for an earlier day."
  (interactive "P")
  (let ((ids (samu--habit-targets)))
    (apply #'samu--habit-enter ids "fail"
           (samu--habit-day-args arg "Not done on: "))))

(defun samu-habit-pause (from until)
  "Pause the habits here from the day FROM to the day UNTIL.
The streak does not break.  Both are read with `org-read-date'; FROM
is today by default, UNTIL the same day."
  (interactive
   (progn
     (require 'org)
     (let ((from (org-read-date nil nil nil "Pause from: ")))
       (list from (org-read-date nil nil nil "Pause until: " nil from)))))
  (let ((ids (samu--habit-targets)))
    (apply #'samu--habit-enter ids "pause"
           "--from" from
           (and until (not (equal until from)) (list "--until" until)))))

(defconst samu--habit-entry-words
  '(("done" . "done") ("fail" . "not done") ("pause" . "pause"))
  "The status of an entry in words.")

(defun samu--habit-entry-text (entry)
  "ENTRY of `samu habit entries' in one line: day, status, amount or until."
  (let ((status (samu--text (samu--get entry 'status))))
    (concat (samu--text (samu--get entry 'day)) "  "
            (or (cdr (assoc status samu--habit-entry-words)) status)
            (pcase status
              ("done" (let ((n (or (samu--get entry 'amount) 1)))
                        (if (> n 1) (format " %d" n) "")))
              ("pause" (if (samu--get entry 'until)
                           (concat " until " (samu--get entry 'until))
                         ""))
              (_ "")))))

(defun samu--habit-last-entry (id)
  "The entry of the habit ID made last, from `samu habit entries', or nil."
  (car (sort (copy-sequence (samu--call (list "habit" "entries" id)))
             (lambda (a b)
               (string> (samu--text (samu--get a 'entry 'created))
                        (samu--text (samu--get b 'entry 'created)))))))

(defun samu--habit-undo-entry (entry)
  "Delete ENTRY, a record of `samu habit entries', after asking."
  (when (y-or-n-p (format "Delete the entry %s? " (samu--habit-entry-text entry)))
    (samu--call (list "habit" "undo" (samu--get entry 'id)))
    (samu--after-action (concat "Deleted: " (samu--habit-entry-text entry)))))

(defun samu-habit-undo ()
  "Delete the entry made last for the habit here: a wrong tap.
On an entry in a habit's buffer, that entry."
  (interactive)
  (let ((entry (get-text-property (point) 'samu-habit-entry)))
    (if entry
        (samu--habit-undo-entry entry)
      (let ((id (car (samu--habit-targets))))
        (samu--habit-undo-entry
         (or (samu--habit-last-entry id)
             (user-error "No entries in the last 28 days")))))))

(defun samu-habit-delete ()
  "On an entry, delete it; elsewhere delete the habit like \\[samu-delete]."
  (interactive)
  (if (get-text-property (point) 'samu-habit-entry)
      (samu-habit-undo)
    (samu-delete)))

;;;;; A habit's buffer

(defconst samu--habit-weeks 12
  "How many weeks the calendar of a habit's buffer shows, as the web.")

(defconst samu--habit-marks
  '(("done" "#" samu-habit-met) ("partial" "+" samu-habit-partial)
    ("fail" "x" samu-habit-fail) ("pause" "=" samu-habit-quiet)
    ("none" "." samu-habit-quiet))
  "The mark and face of a day by its state, the marks of `samu habit show'.")

(defvar samu-habit-mode-map (make-sparse-keymap)
  "Keys of a habit's buffer.")

;; Bound at top level, not in the `defvar': loading the file again
;; then takes new keys, also into buffers already open.
(let ((map samu-habit-mode-map))
  (set-keymap-parent map samu-list-mode-map)
  (define-key map (kbd "RET") #'samu-habit-show-task)
  (define-key map (kbd "DEL") #'samu-habit-delete)
  (define-key map (kbd "<deletechar>") #'samu-habit-delete)
  (define-key map (kbd "<delete>") #'samu-habit-delete))

(define-derived-mode samu-habit-mode samu-list-mode "samu habit"
  "One habit of samudo: goal, streak, rate, the last weeks and the entries.
`h' enters as in the views; DEL on an entry deletes it.

\\{samu-habit-mode-map}")

(with-eval-after-load 'evil
  (evil-set-initial-state 'samu-habit-mode 'emacs))

(defun samu--day-time (day)
  "The noon of DAY, a string YYYY-MM-DD, as a time."
  (encode-time (list 0 0 12
                     (string-to-number (substring day 8 10))
                     (string-to-number (substring day 5 7))
                     (string-to-number (substring day 0 4))
                     nil -1 nil)))

(defun samu--weekday (day)
  "The weekday of DAY, YYYY-MM-DD: 1 for Monday to 7 for Sunday."
  (string-to-number (format-time-string "%u" (samu--day-time day))))

(defun samu--habit-history-days ()
  "How many days back to ask, so the calendar starts on a Monday."
  (+ (* 7 (1- samu--habit-weeks))
     (string-to-number (format-time-string "%u"))))

(defun samu--habit-weeks (history)
  "HISTORY, days oldest first, as weeks from Monday: lists of 7, nil before."
  (let ((weeks nil) (week nil))
    (dolist (day history)
      (let ((wd (samu--weekday (samu--get day 'day))))
        (when (and (= wd 1) week)
          (push (nreverse week) weeks)
          (setq week nil))
        (unless week
          (setq week (make-list (1- wd) nil)))
        (push day week)))
    (when week (push (nreverse week) weeks))
    (nreverse weeks)))

(defun samu--habit-rate (ratio)
  "RATIO of `samu habit show', \"met/of\", as percent and fraction."
  (let* ((parts (split-string (samu--text ratio) "/"))
         (met (string-to-number (or (car parts) "0")))
         (of (string-to-number (or (cadr parts) "0"))))
    (if (> of 0)
        (format "%d %% (%d/%d)" (round (* 100.0 met) of) met of)
      "–")))

(defun samu--insert-habit-calendar (history)
  "Insert HISTORY as a calendar: a line per week, Monday first."
  (insert (make-string 9 ?\s) "Mo Tu We Th Fr Sa Su\n")
  (dolist (week (samu--habit-weeks history))
    (let ((monday (cl-find-if #'identity week)))
      (insert (propertize (format-time-string
                           "  %m-%d  "
                           (time-subtract (samu--day-time (samu--get monday 'day))
                                          (* 86400 (1- (samu--weekday (samu--get monday 'day))))))
                          'face 'samu-meta))
      (dolist (day week)
        (let ((mark (assoc (samu--get day 'state) samu--habit-marks)))
          (insert (cond ((null day) "   ")
                        (mark (concat (propertize (nth 1 mark) 'face (nth 2 mark)
                                                  'help-echo (samu--get day 'day))
                                      "  "))
                        (t ".  ")))))
      (delete-char -2)
      (insert "\n")))
  (insert (propertize "  # done  + partly  x not done  = pause  . nothing"
                      'face 'samu-meta)
          "\n"))

(defun samu--habit-entries (status)
  "The entries in STATUS of `samu habit show', newest first, as records."
  (mapcar (lambda (e)
            `((id . ,(samu--get e 'id)) (day . ,(samu--get e 'day))
              (status . ,(samu--get e 'status)) (amount . ,(samu--get e 'amount))
              (until . ,(samu--get e 'until))
              (by . ,(samu--get e 'created_by 'device))
              (entry . ,e)))
          (sort (copy-sequence (samu--get status 'entries))
                (lambda (a b)
                  (let ((da (samu--text (samu--get a 'day)))
                        (db (samu--text (samu--get b 'day))))
                    (if (equal da db)
                        (string> (samu--text (samu--get a 'created))
                                 (samu--text (samu--get b 'created)))
                      (string> da db)))))))

(defun samu--insert-habit-facts (record)
  "Insert goal, today, streak and rate of the RECORD of `samu habit show'."
  (let* ((status (samu--get record 'status))
         (habit (samu--get status 'task 'habit)))
    (samu--field "Goal" (samu--habit-goal habit))
    (samu--field "Today" (samu--habit-cell record))
    (samu--field "Streak" (format "%d, longest %d"
                                 (or (samu--get record 'streak) 0)
                                 (or (samu--get record 'longest) 0)))
    (samu--field "Rate" (format "4 weeks %s, year %s"
                                 (samu--habit-rate (samu--get record 'recent))
                                 (samu--habit-rate (samu--get record 'year))))
    (samu--field "Relapse" (samu--get record 'last_fail))))

(defun samu--insert-habit-fields (record)
  "Insert the habit part of a task's buffer, from RECORD of `samu habit show'."
  (when record
    (samu--insert-habit-facts record)
    (samu--field "" (propertize "h s shows the history" 'face 'samu-meta))))

(defun samu--fetch-habit ()
  "The record of `samu habit show' a habit's buffer shows."
  (let ((id samu--habit-id))
    (or (car (samu--call (list "habit" "show" id "--days"
                               (number-to-string (samu--habit-history-days)))
                         nil t))
        (user-error "Not a habit: %s" id))))

(defun samu--draw-habit (record)
  "Fill a habit's buffer from RECORD of `samu habit show', point where it was."
  (let* ((inhibit-read-only t)
         (pos (point))
         (id samu--habit-id)
         (status (samu--get record 'status))
         (title (samu--text (samu--get record 'title))))
    (erase-buffer)
    (rename-buffer (format "*samu Habit: %s*" title) t)
    (insert (propertize title 'face 'samu-title) "\n\n")
    (samu--insert-habit-facts record)
    (samu--field "ID" id)
    (samu--heading "History")
    (samu--insert-habit-calendar (samu--get status 'history))
    (samu--heading "Entries")
    (let ((entries (samu--habit-entries status)))
      (if (null entries)
          (insert (propertize "  (none; h h enters one)" 'face 'samu-meta) "\n")
        (dolist (entry entries)
          (let ((start (point)))
            (insert "  " (samu--habit-entry-text entry)
                    (if (samu--get entry 'by)
                        (propertize (concat "  " (samu--text (samu--get entry 'by)))
                                    'face 'samu-meta)
                      "")
                    "\n")
            (put-text-property start (point) 'samu-habit-entry entry)))
        (insert (propertize "  DEL deletes an entry" 'face 'samu-meta) "\n")))
    (goto-char (min pos (point-max)))))

(defun samu-habit-show (id)
  "Show the habit ID: goal, streak, rate, the last weeks, the entries.
Interactively the habit here."
  (interactive (list (car (samu--habit-targets))))
  (let ((buffer (or (cl-find-if (lambda (b) (equal (buffer-local-value 'samu--habit-id b) id))
                                (buffer-list))
                    (generate-new-buffer "*samu Habit*"))))
    (with-current-buffer buffer
      (unless (derived-mode-p 'samu-habit-mode)
        (samu-habit-mode)
        (setq samu--habit-id id))
      (samu--render)
      (goto-char (point-min)))
    (pop-to-buffer buffer)
    (samu--live-ensure)
    buffer))

(defun samu-habit-show-task ()
  "Show the habit's task in its own buffer."
  (interactive)
  (samu-show samu--habit-id))

;;;; A task's buffer

(defvar-local samu--show-logbook nil
  "Whether the task's buffer shows the logbook.")

(defvar samu-task-mode-map (make-sparse-keymap)
  "Keys of a task's buffer; those of the views act on the task or a child.")

;; Bound at top level, not in the `defvar': loading the file again
;; then takes new keys, also into buffers already open.
(let ((map samu-task-mode-map))
  (set-keymap-parent map samu-list-mode-map)
  (define-key map (kbd "RET") #'samu-task-ret)
  (define-key map (kbd "T") #'samu-set-title)
  (define-key map (kbd "N") #'samu-edit-note)
  (define-key map (kbd "C") #'samu-comment)
  (define-key map (kbd "+") #'samu-attach)
  (define-key map (kbd "o") #'samu-open-property)
  (define-key map (kbd "l") #'samu-toggle-logbook))

(define-derived-mode samu-task-mode samu-list-mode "samu task"
  "One task of samudo, with its note, children and comments.

\\{samu-task-mode-map}")

(defun samu--when-text (when)
  "A date or instant of the wire, WHEN, as text."
  (cond ((null when) nil)
        ((stringp when)
         (if (string-match-p "T.*Z\\'" when)
             (format-time-string "%Y-%m-%d %H:%M" (parse-iso8601-time-string when))
           when))
        ((samu--get when 'date))
        ((samu--get when 'at) (samu--when-text (samu--get when 'at)))
        (t (format "%s" when))))

(defun samu--field (name value)
  "Insert the field NAME with VALUE, unless VALUE is empty."
  (unless (string-empty-p (samu--text value))
    (insert (propertize (format "%-15s" name) 'face 'samu-field)
            (samu--text value) "\n")))

(defun samu--heading (text)
  "Insert the heading TEXT of a part of the buffer."
  (insert "\n" (propertize text 'face 'samu-section) "\n"))

(defun samu--note-format (note)
  "Whether NOTE reads as \"org\" or \"markdown\", as samu-core decides (ADR 0058).
Asked once for each text: what a text reads as does not change."
  (or (gethash note samu--note-formats)
      (let ((format (condition-case nil
                        (string-trim (samu--call-text '("render") note))
                      (samu-error nil))))
        (if (member format '("org" "markdown"))
            (puthash note format samu--note-formats)
          "markdown"))))

(defun samu--note-format-is (note format)
  "Remember that NOTE reads as FORMAT, as the CLI said with the note."
  (when (member format '("org" "markdown"))
    (puthash note format samu--note-formats)))

(defun samu--note-mode (format)
  "The major mode for a note in FORMAT."
  (if (equal format "org")
      'org-mode
    (if (fboundp 'markdown-mode) 'markdown-mode 'text-mode)))

(defun samu--fontify (text mode)
  "TEXT fontified as MODE would."
  (if (not (fboundp mode))
      text
    (with-temp-buffer
      (insert text)
      (delay-mode-hooks (funcall mode))
      (font-lock-ensure)
      (buffer-string))))

(defun samu--insert-note (text)
  "Insert the note or comment TEXT, fontified as its format."
  (insert (samu--fontify text (samu--note-mode (samu--note-format text))))
  (unless (bolp) (insert "\n")))

(defun samu--descendants (id kind all)
  "The rows of ALL below the task or project ID of KIND, as a tree."
  (let* ((top (cl-remove-if-not
               (lambda (r) (if (equal kind "project")
                               (and (equal (samu--get (samu--row-task r) 'project) id)
                                    (null (samu--parent r)))
                             (equal (samu--parent r) id)))
               all))
         (ids (mapcar (lambda (r) (samu--get r 'id)) top))
         (more t))
    (while more
      (setq more nil)
      (dolist (r all)
        (when (and (member (samu--parent r) ids) (not (member (samu--get r 'id) ids)))
          (setq ids (append ids (list (samu--get r 'id))) more t))))
    (cl-remove-if-not (lambda (r) (member (samu--get r 'id) ids)) all)))

(defun samu--insert-task-fields (row object)
  "Insert the fields of the task with ROW and OBJECT."
  (let ((project (samu--text (samu--get row 'project))))
    (samu--field "Where" (if (string-empty-p project) "Inbox" (concat "#" project))))
  (samu--field "State" (samu--get row 'state))
  (samu--field "Available" (cond ((samu--row-closed-p row) nil)
                                 ((samu--get row 'available) "yes")
                                 (t (concat "no: " (samu--text (samu--get row 'why))))))
  (samu--field "Completed" (samu--when-text (samu--get object 'completed)))
  (samu--field "Dropped" (samu--when-text (samu--get object 'dropped)))
  (samu--field "Priority" (samu--get object 'priority))
  (samu--field "Flagged" (and (samu--get object 'flagged) "⚑"))
  (samu--field "Tags" (samu--get row 'tags))
  (samu--field "Due" (samu--when-text (samu--get object 'due)))
  (samu--field "Planned" (samu--when-text (samu--get object 'planned)))
  (samu--field "Available from" (samu--when-text (samu--get object 'defer)))
  (samu--field "Estimate" (samu--get object 'estimate))
  (samu--field "Repeat" (samu--text (samu--get row 'next_repeat))))

(defun samu--fetch-task ()
  "What the buffer of a task or project shows, from the CLI, as a plist.
One call, `samu show --full', gives all of it, and with it what the note
and each comment read as.  Also asks for what drawing looks up: the tags
the logbook names."
  (let* ((id samu--task-id)
         (full (or (car (samu--call (append (list "show" id "--full")
                                            (and samu--show-logbook '("--log")))
                                    nil t))
                   (user-error "Not found: %s" id)))
         (shown (samu--get full 'shown))
         (comments (samu--get full 'comments))
         (habits (samu--get full 'habits)))
    (samu--note-format-is (samu--text (samu--get shown 'object 'note))
                          (samu--get full 'note_format))
    (dolist (c comments)
      (samu--note-format-is (samu--text (samu--get c 'body)) (samu--get c 'format)))
    (when samu--show-logbook (samu--tags))
    (list :shown shown
          :row (samu--get full 'row)
          :below (samu--descendants id (samu--get shown 'kind) (samu--get full 'below))
          :habits (and habits
                       (let ((table (make-hash-table :test #'equal)))
                         (dolist (record habits table)
                           (puthash (samu--get record 'id) record table))))
          :habit (samu--get full 'habit)
          :reminders (samu--get full 'reminders)
          :props (samu--get full 'props)
          :comments comments
          :files (samu--get full 'attachments)
          :log (samu--get full 'log))))

(defun samu--insert-task (data)
  "Fill the buffer of a task or project from DATA of `samu--fetch-task'."
  (let* ((id samu--task-id)
         (shown (plist-get data :shown))
         (kind (samu--get shown 'kind))
         (object (samu--get shown 'object))
         (row (plist-get data :row))
         (note (samu--text (samu--get object 'note)))
         (below (plist-get data :below)))
    (setq samu--task-object object
          samu--task-row row
          samu--rows below
          samu--habits (plist-get data :habits))
    (rename-buffer (format "*samu: %s*" (samu--text (samu--get shown 'title))) t)
    (insert (propertize (samu--text (samu--get shown 'title)) 'face 'samu-title) "\n\n")
    (if (equal kind "task")
        (progn (samu--insert-task-fields row object)
               (samu--insert-habit-fields (plist-get data :habit)))
      (samu--field "Kind" (samu--get object 'kind))
      (samu--field "Status" (samu--get object 'status))
      (samu--field "Flagged" (and (samu--get object 'flagged) "⚑"))
      (samu--field "Review" (samu--get object 'review_interval))
      (samu--field "Next review" (samu--get object 'next_review))
      (samu--field "Last reviewed" (samu--get object 'last_reviewed)))
    (samu--field "Reminders"
                 (mapconcat (lambda (r) (samu--when-text (samu--get r 'when)))
                            (plist-get data :reminders) ", "))
    (samu--field "ID" id)
    (let ((props (plist-get data :props)))
      (when props
        (samu--heading "Properties")
        (dolist (p props)
          (let ((start (point)))
            (insert (propertize (format "%-15s" (samu--get p 'key)) 'face 'samu-field)
                    (samu--text (samu--get p 'value))
                    (if (samu--get p 'from)
                        (propertize (format "  (from %s)" (samu--get p 'from)) 'face 'samu-meta)
                      "")
                    "\n")
            (put-text-property start (point) 'samu-property (samu--get p 'key))))))
    (samu--heading "Note")
    (if (string-blank-p note)
        (insert (propertize "(none; N writes one)" 'face 'samu-meta) "\n")
      (samu--insert-note note))
    (when below
      (samu--heading "Subtasks")
      (samu--insert-rows below))
    (let ((comments (plist-get data :comments)))
      (when comments
        (samu--heading "Comments")
        (dolist (c comments)
          (insert (propertize (format "%s, %s" (samu--text (samu--get c 'author))
                                      (samu--when-text (samu--get c 'created)))
                              'face 'samu-meta)
                  "\n")
          (samu--insert-note (samu--text (samu--get c 'body)))
          (insert "\n"))))
    (let ((files (plist-get data :files)))
      (when files
        (samu--heading "Attachments")
        (dolist (a files)
          (let ((start (point)))
            (insert "  " (samu--text (or (samu--get a 'name) (samu--get a 'short)))
                    (propertize (format "  %s  %s" (samu--text (samu--get a 'media_type))
                                        (file-size-human-readable (or (samu--get a 'size) 0)))
                                'face 'samu-meta)
                    "\n")
            (put-text-property start (point) 'samu-attachment a)))))
    (if (not samu--show-logbook)
        (insert "\n" (propertize "Logbook: l shows it" 'face 'samu-meta) "\n")
      (samu--heading "Logbook")
      (dolist (entry (plist-get data :log))
        (insert (propertize (samu--text (samu--when-text (samu--get entry 'at))) 'face 'samu-meta)
                (format "  %s  %s" (samu--text (samu--get entry 'device))
                        (samu--text (samu--get entry 'kind)))
                (let ((old (samu--get entry 'old)) (new (samu--get entry 'new)))
                  (if (or old new)
                      (format "  %s → %s" (samu--log-value old) (samu--log-value new))
                    ""))
                "\n")))))

(defun samu--log-value (value)
  "VALUE of a logbook entry, short; a tag's id as its path."
  (cond ((null value) "–")
        ((and (stringp value) (samu--tag-path value)))
        (t (truncate-string-to-width (samu--text value) 40 nil nil "…"))))

(defun samu--draw-task (data)
  "Fill the task's buffer from DATA of `samu--fetch-task', point where it was."
  (let ((inhibit-read-only t)
        (pos (point)))
    (erase-buffer)
    (samu--insert-task data)
    (goto-char (min pos (point-max)))))

;;;###autoload
(defun samu-show (id &optional other-window)
  "Show the task or project ID in its own buffer; OTHER-WINDOW elsewhere."
  (interactive (list (samu--read-task "Task: ")))
  (let ((buffer (cl-find-if (lambda (b) (equal (buffer-local-value 'samu--task-id b) id))
                            (buffer-list))))
    (unless buffer
      (setq buffer (generate-new-buffer "*samu: …*"))
      (with-current-buffer buffer
        (samu-task-mode)
        (setq samu--task-id id)))
    (with-current-buffer buffer
      (samu--render))
    (if other-window
        (display-buffer buffer)
      (pop-to-buffer-same-window buffer))
    (samu--live-ensure)
    buffer))

(defun samu--show-object (id kind)
  "Open the object ID of KIND: tasks and projects here, tags and folders as views."
  (pcase kind
    ("tag" (samu-tag (or (samu--tag-path id) id)))
    ("folder" (samu-projects id))
    (_ (samu-show id))))

(defun samu-show-at-point ()
  "Show the task here; on a project, its tasks."
  (interactive)
  (let ((row (get-text-property (point) 'samu-row)))
    (cond ((null row) (user-error "No task here"))
          ((equal (samu--get row 'kind) "project") (samu-project (samu--get row 'id)))
          ((null (samu--get row 'id)) (user-error "An appointment, not a task"))
          (t (samu-show (samu--get row 'id))))))

(defun samu-show-other-window ()
  "Show the task here in another window; point stays."
  (interactive)
  (samu-show (or (get-text-property (point) 'samu-id) (user-error "No task here"))
             t))

(defun samu-task-ret ()
  "On a child, show it; on an attachment or a property, open it."
  (interactive)
  (cond ((get-text-property (point) 'samu-attachment) (samu-open-attachment))
        ((get-text-property (point) 'samu-property) (samu-open-property))
        ((get-text-property (point) 'samu-id) (samu-show-at-point))))

(defun samu-toggle-logbook ()
  "Show or hide the logbook."
  (interactive)
  (setq samu--show-logbook (not samu--show-logbook))
  (samu--render))

(defun samu-set-title (title)
  "Give the task here the TITLE."
  (interactive (list (read-string "Title: " (samu--title-at-point))))
  (samu--call (list "set" (samu--id-at-point) "title" (json-encode title)))
  (samu--after-action nil))

(defun samu-open-property ()
  "Open the property on this line with `samu open' (ADR 0054)."
  (interactive)
  (samu--call-text (list "open" samu--task-id
                         (or (get-text-property (point) 'samu-property)
                             (user-error "No property here")))))

(defun samu-open-attachment ()
  "Fetch the attachment on this line into the cache and open it."
  (interactive)
  (let* ((a (or (get-text-property (point) 'samu-attachment)
                (user-error "No attachment here")))
         (dir (expand-file-name (concat "samu/" (samu--get a 'blob))
                                (or (getenv "XDG_CACHE_HOME") "~/.cache")))
         (file (expand-file-name (file-name-nondirectory
                                  (samu--text (or (samu--get a 'name) (samu--get a 'short))))
                                 dir)))
    (unless (file-exists-p file)
      (make-directory dir t)
      (samu--call-text (list "blob" "get" (samu--get a 'blob) "--output" file)))
    (find-file-other-window file)))

(defun samu-attach (file)
  "Attach FILE to the task here."
  (interactive "fAttach: ")
  (samu--call (list "attach" (samu--id-at-point) (expand-file-name file)))
  (samu--after-action (format "Attached: %s" (file-name-nondirectory file))))

;;;; Writing a note or a comment

(defvar-local samu--edit-save nil
  "What `samu-edit-save' does with the text.")

(defvar samu-edit-mode-map (make-sparse-keymap)
  "Keys while writing a note or comment.")

;; Bound at top level, not in the `defvar': loading the file again
;; then takes new keys, also into buffers already open.
(let ((map samu-edit-mode-map))
  (define-key map (kbd "C-c C-c") #'samu-edit-save)
  (define-key map (kbd "C-c C-k") #'samu-edit-cancel))

(define-minor-mode samu-edit-mode
  "Writing a note or comment of samudo: \\[samu-edit-save] saves, \\[samu-edit-cancel] cancels."
  :lighter " samu")

(defun samu--edit-buffer (name text mode save)
  "Open a buffer NAME with TEXT in MODE; \\[samu-edit-save] calls SAVE with the text."
  (let ((buffer (generate-new-buffer name)))
    (with-current-buffer buffer
      (insert text)
      (funcall (if (fboundp mode) mode #'text-mode))
      (samu-edit-mode 1)
      (setq samu--edit-save save)
      (set-buffer-modified-p nil)
      (setq header-line-format
            (substitute-command-keys
             "\\<samu-edit-mode-map>\\[samu-edit-save] saves, \\[samu-edit-cancel] discards"))
      (goto-char (point-min)))
    (pop-to-buffer buffer)
    buffer))

(defun samu-edit-save ()
  "Save what is written and close the buffer."
  (interactive)
  (funcall samu--edit-save (string-trim-right (buffer-string)))
  (quit-window t)
  (samu--after-action "Saved"))

(defun samu-edit-cancel ()
  "Close the buffer without saving."
  (interactive)
  (when (or (not (buffer-modified-p)) (yes-or-no-p "Discard? "))
    (quit-window t)))

(defun samu--save-note (id text)
  "Make TEXT the note of ID; empty takes the note off."
  (if (string-blank-p text)
      (samu--call (list "unset" id "note"))
    ;; JSON, so a note like `true' or `42' stays text
    (samu--call (list "set" id "note" (json-encode text)))))

(defun samu-edit-note ()
  "Write the note of the task here, in the mode of its format."
  (interactive)
  (let* ((id (or (samu--id-at-point) (user-error "No task here")))
         (object (if (equal id samu--task-id)
                     samu--task-object
                   (samu--get (car (samu--call (list "show" id))) 'object)))
         (note (samu--text (samu--get object 'note))))
    (samu--edit-buffer (format "*samu note: %s*" (samu--text (samu--get object 'title)))
                       note
                       (samu--note-mode (if (string-blank-p note) "org" (samu--note-format note)))
                       (lambda (text) (samu--save-note id text)))))

(defun samu-comment ()
  "Write a comment on the task here (ADR 0039 §3)."
  (interactive)
  (let ((id (or (samu--id-at-point) (user-error "No task here"))))
    (samu--edit-buffer "*samu comment*" "" (samu--note-mode "markdown")
                       (lambda (text)
                         (unless (string-blank-p text)
                           (samu--call (list "comment" id "-") text))))))

;;;; Links

(defconst samu--id-regexp
  "\\`/*\\([0-9a-fA-F]\\{8\\}-[0-9a-fA-F]\\{4\\}-[0-9a-fA-F]\\{4\\}-[0-9a-fA-F]\\{4\\}-[0-9a-fA-F]\\{12\\}\\)/*\\'"
  "What follows `samu:' in a link: one id.")

(defun samu--id (path)
  "The id in PATH, the part of a `samu:' link after the colon, or nil."
  (when (string-match samu--id-regexp path)
    (downcase (match-string 1 path))))

(defun samu--url (id)
  "The web UI's address of the object ID.
With `samu-profile' the CLI says it: the profile's own web UI, through
the front while it answers (ADR 0087 §5).  Else `samu-web-url'."
  (if samu-profile
      (nth 1 (samu--get (car (samu--call (list "open-uri" (concat "samu:" id) "--print")))
                        'command))
    (unless samu-web-url
      (user-error "Set `samu-web-url' or `samu-profile' to open samu links in the browser"))
    (format "%s/o/%s" (string-trim-right samu-web-url "/+") id)))

(defun samu--link-in-browser-p (arg)
  "Whether a link followed with prefix ARG opens in the browser."
  (if arg (eq samu-link-target 'emacs) (eq samu-link-target 'browser)))

(defun samu-follow-link (path &optional arg)
  "Open the object of a `samu:PATH' link here, or with ARG in the web UI.
`samu-link-target' says which is the default.  The browser goes
through `browse-url', not `samu open-uri': a daemon started without
the desktop's environment leaves `xdg-open' without a browser."
  (let ((id (or (samu--id path) (user-error "samu:%s is no samu address" path))))
    (if (samu--link-in-browser-p arg)
        (browse-url (samu--url id))
      (samu--show-object id (samu--get (car (samu--call (list "show" id))) 'kind)))))

(defun samu-export-link (path description backend &optional _info)
  "Export a `samu:PATH' link with DESCRIPTION for BACKEND as a web link.
Without `samu-web-url' or `samu-profile' it stays a `samu:' link."
  (let* ((id (samu--id path))
         (url (if (and id (or samu-profile samu-web-url))
                  (samu--url id)
                (concat "samu:" path)))
         (text (or description url)))
    (pcase backend
      ((or 'html '11ty) (format "<a href=\"%s\">%s</a>" url text))
      ('md (format "[%s](%s)" text url))
      ('latex (format "\\href{%s}{%s}" url text))
      ('ascii (if description (format "%s (%s)" text url) url))
      (_ nil))))

(defun samu-store-link (&optional _interactive)
  "Store a `samu:' link to the task here, for `org-insert-link'."
  (when (derived-mode-p 'samu-list-mode)
    (let ((id (samu--id-at-point)))
      (when id
        (org-link-store-props
         :type "samu" :link (concat "samu:" id)
         :description (samu--title-at-point))
        t))))

(defun samu-complete-link (&optional _arg)
  "A `samu:' link to an open task, read with completion."
  (concat "samu:" (samu--read-task "Task: ")))

(with-eval-after-load 'ol
  (org-link-set-parameters "samu"
                           :follow #'samu-follow-link
                           :export #'samu-export-link
                           :store #'samu-store-link
                           :complete #'samu-complete-link))

(provide 'samu)
;;; samu.el ends here
