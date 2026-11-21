;;; test-samu.el --- Tests for samu.el -*- lexical-binding: t; -*-

;; SPDX-FileCopyrightText: 2026 Marc Seibert
;; SPDX-License-Identifier: AGPL-3.0-or-later

;;; Commentary:

;; No CLI and no server: `samu--run' is replaced by answers given as
;; synthetic JSON, and every call is recorded.
;;
;;   emacs -Q --batch -L emacs -l emacs/samu.el -l emacs/test-samu.el \
;;     -f ert-run-tests-batch-and-exit

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'samu)

;; No test starts `samu events' unless it says so.
(setq samu-live nil)

;; Every call waits for its answer unless a test says otherwise.
(setq samu--sync t)

(defvar samu-test--calls nil
  "The calls of the CLI, newest first, as (ARGS INPUT).")

(defun samu-test--answer (answers args)
  "The answer in ANSWERS for ARGS: the first whose key starts ARGS."
  (or (cl-find-if (lambda (answer)
                    (let ((key (car answer)))
                      (equal key (cl-subseq args 0 (min (length key) (length args))))))
                  answers)
      '(nil 0 "[]" "")))

(defmacro samu-test--with-cli (answers &rest body)
  "Run BODY with the CLI answering from ANSWERS.
Each answer is (ARGS-PREFIX CODE STDOUT STDERR)."
  (declare (indent 1))
  `(let ((samu-test--calls nil)
         (samu--cache nil)
         (samu--note-formats (make-hash-table :test #'equal))
         (samu-profile nil)
         (samu-server nil)
         (samu-account nil)
         (samu-scope nil))
     (cl-letf (((symbol-function 'samu--run)
                (lambda (args &optional input)
                  (push (list args input) samu-test--calls)
                  (cdr (samu-test--answer ,answers args)))))
       ,@body)))

(defun samu-test--called (prefix)
  "The args of the calls that start with PREFIX, oldest first."
  (cl-loop for (args _input) in (reverse samu-test--calls)
           when (equal prefix (cl-subseq args 0 (min (length prefix) (length args))))
           collect args))

(defconst samu-test--tags
  "[{\"id\":\"w1\",\"key\":null,\"marks\":\"workflow\",\"path\":\"Entwicklung\"},
    {\"id\":\"s1\",\"key\":\"w\",\"marks\":\"\",\"path\":\"Entwicklung/Wunsch\"},
    {\"id\":\"s2\",\"key\":\"a\",\"marks\":\"\",\"path\":\"Entwicklung/Aufgabe\"},
    {\"id\":\"t1\",\"key\":null,\"marks\":\"\",\"path\":\"agenda\"},
    {\"id\":\"t2\",\"key\":null,\"marks\":\"\",\"path\":\"Ort\"},
    {\"id\":\"t3\",\"key\":null,\"marks\":\"\",\"path\":\"Ort/Büro\"}]"
  "Synthetic tags: one workflow with two states, three plain tags.")

(defun samu-test--row (id title &rest fields)
  "A row of `samu list' for ID with TITLE and FIELDS, a plist of task fields."
  (let ((task `((id . ,id) (title . ,title)
                ,@(cl-loop for (k v) on fields by #'cddr
                           collect (cons (intern (substring (symbol-name k) 1)) v)))))
    `((id . ,id) (short . ,(substring id 0 (min 8 (length id))))
      (title . ,title) (available . t) (project . "")
      (status . ((task . ,task) (workflow . "w1"))))))

;;;; Calling the CLI

(ert-deftest samu-test-parse ()
  (should (equal (samu--parse "[{\"a\":null,\"b\":false,\"c\":[1]}]")
                 '(((a) (b) (c 1)))))
  (should (null (samu--parse "  \n"))))

(ert-deftest samu-test-call-adds-format-and-globals ()
  (samu-test--with-cli '((("list") 0 "[{\"id\":\"x\"}]" ""))
    (let ((samu-server "http://127.0.0.1:1") (samu-account "privat"))
      (should (equal (samu--call '("list")) '(((id . "x")))))
      (should (equal (car (car samu-test--calls))
                     '("list" "--format" "json"))))))

(ert-deftest samu-test-run-passes-server-and-account ()
  (let ((samu-server "http://127.0.0.1:1") (samu-account "privat") (seen nil))
    (cl-letf (((symbol-function 'call-process)
               (lambda (_program _infile _dest _display &rest args)
                 (setq seen args) 0)))
      (samu--run '("health")))
    (should (equal seen '("--server" "http://127.0.0.1:1" "--account" "privat" "health")))))

(ert-deftest samu-test-run-passes-the-profile-first ()
  (let ((samu-profile "dienst") (samu-server nil) (samu-account nil) (seen nil))
    (cl-letf (((symbol-function 'call-process)
               (lambda (_program _infile _dest _display &rest args)
                 (setq seen args) 0)))
      (samu--run '("health")))
    (should (equal seen '("--profile" "dienst" "health")))))

(ert-deftest samu-test-switch-profile-clears-server-and-account ()
  (let ((samu-profile nil) (samu-server "http://x") (samu-account "privat")
        (switched nil))
    (cl-letf (((symbol-function 'samu--switched) (lambda () (setq switched t))))
      (samu-switch-profile "dienst")
      (should (equal samu-profile "dienst"))
      (should (null samu-server))
      (should (null samu-account))
      (should switched)
      (samu-switch-profile "")
      (should (null samu-profile)))))

(ert-deftest samu-test-profile-names-ask-without-a-profile ()
  ;; The list comes from the CLI's own settings, not the profile chosen.
  (let ((samu-profile "dienst") (samu-server "http://x") (seen 'unset))
    (cl-letf (((symbol-function 'samu--run)
               (lambda (_args &optional _input)
                 (setq seen (samu--global-args))
                 (list 0 "[{\"name\":\"privat\"},{\"name\":\"dienst\"}]" ""))))
      (should (equal (samu--profile-names) '("privat" "dienst"))))
    (should (null seen))))

(ert-deftest samu-test-switching-restarts-the-live-stream ()
  (let ((calls nil))
    (cl-letf (((symbol-function 'samu--live-running-p) (lambda () t))
              ((symbol-function 'samu--live-stop) (lambda () (push 'stop calls)))
              ((symbol-function 'samu--live-start) (lambda () (push 'start calls)))
              ((symbol-function 'samu--refresh-all) #'ignore)
              ((symbol-function 'samu--clock-update) #'ignore))
      (samu--switched))
    (should (equal (reverse calls) '(stop start)))))

(ert-deftest samu-test-failure-names-stderr-once ()
  (samu-test--with-cli '((("show") 1 "" "warning\nsamu: no such task"))
    (let ((err (should-error (samu--call '("show" "x")) :type 'samu-error)))
      (should (equal (cadr err) "samu: no such task"))
      ;; what a key that waits shows: the line once, without quotes
      (should (equal (error-message-string err) "samu: no such task")))))

(ert-deftest samu-test-not-found-is-an-answer-where-allowed ()
  (samu-test--with-cli '((("clock" "out") 3 "" "samu: no clock runs"))
    (should (null (samu--call '("clock" "out") nil t)))
    (should-error (samu--call '("clock" "out")) :type 'samu-error)))

(ert-deftest samu-test-scope-args ()
  (let ((samu-scope nil)) (should (null (samu--scope-args))))
  (let ((samu-scope 'none)) (should (equal (samu--scope-args) '("--no-scope"))))
  (let ((samu-scope "Arbeit")) (should (equal (samu--scope-args) '("--scope" "Arbeit")))))

;;;; Tags and workflows

(ert-deftest samu-test-plain-tags-leave-out-workflows-and-states ()
  (samu-test--with-cli `((("tag" "list") 0 ,samu-test--tags ""))
    (should (equal (samu--plain-tag-paths) '("agenda" "Ort" "Ort/Büro")))
    (should (equal (samu--workflow-states "w1") '(("w" "Wunsch") ("a" "Aufgabe"))))
    (should (equal (samu--tag-path "t3") "Ort/Büro"))))

(ert-deftest samu-test-tag-changes ()
  (should (equal (samu--tag-changes '("agenda" "Ort") '("Ort" "neu"))
                 '("--add" "neu" "--remove" "agenda")))
  (should (null (samu--tag-changes '("a") '("a")))))

(ert-deftest samu-test-set-tags-makes-a-new-tag-and-the-one-above-first ()
  (samu-test--with-cli `((("tag" "list") 0 ,samu-test--tags ""))
    (with-temp-buffer
      (samu-list-mode)
      (setq samu--task-id "x"
            samu--task-row (samu-test--row "x" "Milch" :tags '(((tag . "t1")))))
      (cl-letf (((symbol-function 'completing-read-multiple)
                 (lambda (&rest _) '("agenda" "Haus/Keller")))
                ((symbol-function 'samu--after-action) #'ignore))
        (samu-set-tags))
      (should (equal (samu-test--called '("tag" "add"))
                     '(("tag" "add" "Haus" "--format" "json")
                       ("tag" "add" "Haus/Keller" "--format" "json"))))
      (should (equal (samu-test--called '("bulk"))
                     '(("bulk" "tag" "--add" "Haus/Keller" "x" "--format" "json")))))))

;;;; Rows and views

(ert-deftest samu-test-by-project-puts-the-inbox-first ()
  (let ((groups (samu--by-project
                 (list '((id . "1") (project . "Garten"))
                       '((id . "2") (project . ""))
                       '((id . "3") (project . "Garten"))))))
    (should (equal (mapcar #'car groups) '("Inbox" "Garten")))
    (should (equal (mapcar (lambda (r) (alist-get 'id r)) (cdr (assoc "Garten" groups)))
                   '("1" "3")))))

(ert-deftest samu-test-children-are-indented-below-their-parent ()
  (with-temp-buffer
    (samu--insert-rows (list (samu-test--row "p" "Eltern")
                             (samu-test--row "c" "Kind" :parent "p")
                             (samu-test--row "g" "Enkel" :parent "c")
                             (samu-test--row "o" "Waise" :parent "nicht-hier")))
    (should (equal (buffer-string) "  Eltern\n    Kind\n      Enkel\n  Waise\n"))
    (goto-char (point-min))
    (forward-line 2)
    (should (equal (get-text-property (point) 'samu-id) "g"))))

(ert-deftest samu-test-row-line-shows-what-is-set ()
  (let ((row (append '((state . "Aufgabe") (priority . "A") (flag . "!")
                       (project . "Haushalt") (tags . "@agenda")
                       (due_text . "2026-11-12") (due . "overdue"))
                     (samu-test--row "x" "Milch"))))
    (should (equal (substring-no-properties (samu--row-line row 0 t))
                   "  Aufgabe !A ⚑ Milch  #Haushalt  @agenda  2026-11-12"))
    (should (eq (get-text-property 0 'face (samu--row-line '((title . "x") (available)) 0))
                nil))))

(ert-deftest samu-test-unavailable-and-closed-are-shown-so ()
  (let ((blocked (samu-test--row "x" "Wartet"))
        (done (samu-test--row "y" "Fertig" :completed "2026-11-12T10:00:00Z")))
    (setf (alist-get 'available blocked) nil)
    (should (eq (get-text-property 2 'face (samu--row-line blocked 0)) 'samu-unavailable))
    (should (eq (get-text-property 2 'face (samu--row-line done 0)) 'samu-done))
    (should (samu--row-closed-p done))))

(ert-deftest samu-test-forecast-joins-the-list-and-keeps-appointments ()
  (let ((open (make-hash-table :test #'equal)))
    (puthash "x" (append '((why . "")) (samu-test--row "x" "Milch")) open)
    (let ((row (samu--forecast-row '((id . "x") (why . "fällig") (title . "Milch")) open))
          (meeting (samu--forecast-row '((id . "m") (why . "termin") (date . "09:00")
                                         (title . "Treffen"))
                                       open)))
      (should (equal (alist-get 'why row) "fällig"))
      (should (alist-get 'status row))
      (should (equal (alist-get 'title meeting) "09:00 Treffen"))
      (should (null (alist-get 'id meeting))))))

(ert-deftest samu-test-section-title ()
  (let ((system-time-locale "C"))
    (should (equal (samu--section-title "2026-11-12") "Thursday 2026-11-12")))
  (should (equal (samu--section-title "überfällig") "Überfällig")))

(ert-deftest samu-test-inbox-is-the-tasks-without-project ()
  (samu-test--with-cli
      '((("list") 0 "[{\"id\":\"1\",\"project\":\"\"},{\"id\":\"2\",\"project\":\"Garten\"}]" ""))
    (should (equal (mapcar (lambda (r) (alist-get 'id r)) (samu--inbox-rows)) '("1")))))

(ert-deftest samu-test-a-view-keeps-point-on-its-task ()
  (with-temp-buffer
    (samu-list-mode)
    (let ((rows (list (samu-test--row "a" "Eins") (samu-test--row "b" "Zwei"))))
      (setq samu--view (list :title "Test" :fetch (lambda () (list (cons "G" rows)))))
      (samu--render)
      (should (equal (get-text-property (point) 'samu-id) "a"))
      (samu-next-task 1)
      (should (equal (get-text-property (point) 'samu-id) "b"))
      (setq rows (list (samu-test--row "c" "Neu") (samu-test--row "a" "Eins")
                       (samu-test--row "b" "Zwei")))
      (samu--render)
      (should (equal (get-text-property (point) 'samu-id) "b")))))

;;;; Actions

(ert-deftest samu-test-done-reopens-only-when-all-are-closed ()
  (samu-test--with-cli nil
    (with-temp-buffer
      (samu-list-mode)
      (setq samu--rows (list (samu-test--row "a" "Offen")
                             (samu-test--row "b" "Zu" :completed "2026-11-12T10:00:00Z"))
            samu--marked '("b" "a"))
      (cl-letf (((symbol-function 'samu--refresh-all) #'ignore))
        (samu-toggle-done)
        (setq samu--marked '("b"))
        (samu-toggle-done))
      (should (equal (samu-test--called '("bulk"))
                     '(("bulk" "done" "a" "b" "--format" "json")
                       ("bulk" "reopen" "b" "--format" "json")))))))

;; Exit code 7 of `samu bulk done' (ADR 0128): the waiting task, its
;; open tasks below indented, then the error.
(defconst samu-test--needs-choice
  "0000000a Gruppe\n  0000000c Rest\n  0000000d Noch einer\nsamu: a task has open tasks below; nothing changed. Give --children close or --children lift")

(defconst samu-test--needs-choice-one
  "0000000a Gruppe\n  0000000c Rest\nsamu: a task has open tasks below; nothing changed. Give --children close or --children lift")

(ert-deftest samu-test-done-with-open-children-asks-and-lifts ()
  (samu-test--with-cli `((("bulk" "done" "a") 7 "" ,samu-test--needs-choice))
    (with-temp-buffer
      (samu-list-mode)
      (setq samu--rows (list (samu-test--row "a" "Gruppe")))
      (let (question)
        (cl-letf (((symbol-function 'samu--refresh-all) #'ignore)
                  ((symbol-function 'read-multiple-choice)
                   (lambda (prompt choices &rest _)
                     (setq question prompt)
                     (assq ?l choices))))
          (setq samu--marked '("a"))
          (samu-toggle-done))
        (should (equal question "2 open subtasks. What happens to them? ")))
      (should (equal (samu-test--called '("bulk"))
                     '(("bulk" "done" "a" "--format" "json")
                       ("bulk" "done" "--children" "lift" "a" "--format" "json")))))))

(ert-deftest samu-test-done-with-open-children-can-be-cancelled ()
  (samu-test--with-cli `((("bulk" "done") 7 "" ,samu-test--needs-choice))
    (with-temp-buffer
      (samu-list-mode)
      (setq samu--rows (list (samu-test--row "a" "Gruppe") (samu-test--row "b" "Andere"))
            samu--marked '("b" "a"))
      (let (question)
        (cl-letf (((symbol-function 'samu--refresh-all) #'ignore)
                  ((symbol-function 'read-multiple-choice)
                   (lambda (prompt choices &rest _)
                     (setq question prompt)
                     (assq ?c choices))))
          (samu-toggle-done))
        (should (equal question
                       "1 of the 2 tasks have open subtasks. What happens to them? ")))
      (should (equal (samu-test--called '("bulk"))
                     '(("bulk" "done" "a" "b" "--format" "json"))))
      (should (equal samu--marked '("b" "a"))))))

(ert-deftest samu-test-drop-with-open-children-drops-them-along ()
  (samu-test--with-cli `((("bulk" "drop" "a") 7 "" ,samu-test--needs-choice-one))
    (with-temp-buffer
      (samu-list-mode)
      (setq samu--task-id "a" samu--task-row (samu-test--row "a" "Gruppe"))
      (let (along question)
        (cl-letf (((symbol-function 'samu--refresh-all) #'ignore)
                  ((symbol-function 'yes-or-no-p) (lambda (_) t))
                  ((symbol-function 'read-multiple-choice)
                   (lambda (prompt choices &rest _)
                     (setq along (cadr (assq ?t choices)) question prompt)
                     (assq ?t choices))))
          (samu-drop))
        (should (equal along "drop them too"))
        (should (equal question "1 open subtask. What happens to it? ")))
      (should (equal (samu-test--called '("bulk"))
                     '(("bulk" "drop" "a" "--format" "json")
                       ("bulk" "drop" "--children" "close" "a" "--format" "json")))))))

(ert-deftest samu-test-close-fails-on-other-errors ()
  (samu-test--with-cli '((("bulk" "done") 4 "" "samu: no server"))
    (with-temp-buffer
      (samu-list-mode)
      (setq samu--task-id "a" samu--task-row (samu-test--row "a" "Gruppe"))
      (should-error (samu-toggle-done) :type 'samu-error))))

(ert-deftest samu-test-pick-state-sends-the-key ()
  (samu-test--with-cli `((("tag" "list") 0 ,samu-test--tags ""))
    (with-temp-buffer
      (samu-list-mode)
      (setq samu--task-id "x" samu--task-row (samu-test--row "x" "Milch"))
      (cl-letf (((symbol-function 'read-char-choice) (lambda (&rest _) ?a))
                ((symbol-function 'samu--refresh-all) #'ignore))
        (samu-pick-state))
      (should (equal (samu-test--called '("bulk"))
                     '(("bulk" "state" "--to" "a" "x" "--format" "json")))))))

(ert-deftest samu-test-priority-space-takes-it-off ()
  (samu-test--with-cli nil
    (with-temp-buffer
      (samu-list-mode)
      (setq samu--task-id "x")
      (cl-letf (((symbol-function 'samu--refresh-all) #'ignore)
                ((symbol-function 'read-char-choice) (lambda (&rest _) ?b)))
        (samu-set-priority))
      (cl-letf (((symbol-function 'samu--refresh-all) #'ignore)
                ((symbol-function 'read-char-choice) (lambda (&rest _) ?\s)))
        (samu-set-priority))
      (should (equal (mapcar #'car (reverse samu-test--calls))
                     '(("set" "x" "priority" "B" "--format" "json")
                       ("unset" "x" "priority" "--format" "json")))))))

(ert-deftest samu-test-a-note-goes-as-json-so-it-stays-text ()
  (samu-test--with-cli nil
    (samu--save-note "x" "true")
    (samu--save-note "x" "  ")
    (should (equal (mapcar #'car (reverse samu-test--calls))
                   '(("set" "x" "note" "\"true\"" "--format" "json")
                     ("unset" "x" "note" "--format" "json"))))))

(ert-deftest samu-test-marking-draws-and-forgets ()
  (with-temp-buffer
    (samu-list-mode)
    (setq samu--view (list :title "T" :fetch
                           (lambda () (list (cons nil (list (samu-test--row "a" "Eins")
                                                            (samu-test--row "b" "Zwei")))))))
    (samu--render)
    (samu-mark)
    (should (equal samu--marked '("a")))
    (should (equal (get-text-property (point) 'samu-id) "b"))
    (forward-line -1)
    (should (looking-at "> Eins"))
    (should (equal (get-text-property (point) 'samu-id) "a"))
    (should (equal (samu--targets) '("a")))
    (samu-unmark)
    (should (null samu--marked))
    (forward-line -1)
    (should (looking-at "  Eins"))))

;;;; Capture

(ert-deftest samu-test-capture-text ()
  (should (equal (samu--capture-text "Milch" "") "Milch"))
  (should (equal (samu--capture-text "Milch #Haus" "aus der Mail")
                 "Milch #Haus // aus der Mail"))
  (should (equal (samu--capture-text "Milch // schon da" "mehr")
                 "Milch // schon da\n\nmehr")))

(ert-deftest samu-test-capture-sends-parent-and-note ()
  (samu-test--with-cli '((("add") 0 "[{\"title\":\"Kind\",\"short\":\"12345678\",\"warnings\":[]}]" ""))
    (with-temp-buffer
      (insert "aus dem Text")
      (setq buffer-file-name "/tmp/synthetisch.txt")
      (transient-mark-mode 1)
      (push-mark (point-min) t t)
      (goto-char (point-max))
      (cl-letf (((symbol-function 'samu--read-line) (lambda (&rest _) "Kind @agenda"))
                ((symbol-function 'samu--refresh-all) #'ignore))
        (samu-capture "p"))
      (setq buffer-file-name nil))
    (should (equal (car (samu-test--called '("add")))
                   '("add" "--parent" "p"
                     "Kind @agenda // aus dem Text\n\n[[file:/tmp/synthetisch.txt::1][synthetisch.txt]]"
                     "--format" "json")))))

(ert-deftest samu-test-capture-lines ()
  (should (samu--several-p "Milch\n\n  Brot"))
  (should-not (samu--several-p "Milch\n  \n"))
  (samu-test--with-cli '((("add" "--lines") 0 "[{\"short\":\"1\"},{\"short\":\"2\"}]" ""))
    (cl-letf (((symbol-function 'samu--read-line) (lambda (&rest _) "Milch\n  - Brot"))
              ((symbol-function 'samu--refresh-all) #'ignore))
      (should (= 2 (length (samu-capture "p")))))
    (should (equal (car (samu-test--called '("add")))
                   '("add" "--lines" "--parent" "p" "-" "--format" "json")))
    (should (equal (cadr (car samu-test--calls)) "Milch\n  - Brot"))))

(ert-deftest samu-test-capture-lines-in-a-buffer ()
  (samu-test--with-cli '((("add" "--lines") 0 "[{\"short\":\"1\"},{\"short\":\"2\"}]" ""))
    (cl-letf (((symbol-function 'samu--refresh-all) #'ignore))
      (let ((buffer (samu-capture-lines nil "p")))
        (with-current-buffer buffer
          (should (eq major-mode 'samu-lines-mode))
          (should (eq (key-binding (kbd "C-c C-c")) #'samu-lines-finish))
          (insert "Einkauf\n  Milch\n")
          (should (= 2 (length (samu-lines-finish)))))
        (should-not (buffer-live-p buffer))))
    (should (equal (car (samu-test--called '("add")))
                   '("add" "--lines" "--parent" "p" "-" "--format" "json")))
    (should (equal (cadr (car samu-test--calls)) "Einkauf\n  Milch\n"))))

(ert-deftest samu-test-capture-completion ()
  (samu-test--with-cli `((("projects") 0 "[{\"id\":\"p\",\"title\":\"Garten\"}]" "")
                         (("tag" "list") 0 ,samu-test--tags "")
                         (("template" "list") 0 "[{\"call\":\"/inbox\",\"title\":\"inbox\"}]" ""))
    (should (equal (samu--capture-candidates ?#) '("#Garten")))
    (should (member "@Ort/Büro" (samu--capture-candidates ?@)))
    (should-not (member "@Entwicklung/Wunsch" (samu--capture-candidates ?@)))
    (should (equal (samu--capture-candidates ?/) '("/inbox")))
    (with-temp-buffer
      (insert "Milch #Ga")
      (should (equal (cl-subseq (samu--capture-capf) 0 2) '(7 10))))
    (with-temp-buffer
      (insert "Milch // notiz")
      (should (null (samu--capture-capf))))))

;; ADR 0084 §5: a template chosen by its key, `h' and `H' two keys, starts
;; the line with its call and point in front.
(ert-deftest samu-test-capture-template-by-key ()
  (samu-test--with-cli
      '((("template" "list") 0
         "[{\"call\":\"/habit\",\"title\":\"habit\",\"key\":\"h\",\"text\":\"@habit\"},{\"call\":\"/inbox\",\"title\":\"inbox\",\"key\":null},{\"call\":\"/Hund\",\"title\":\"Hund\",\"key\":\"H\"}]" "")
        (("add") 0 "[{\"title\":\"Wauwau\",\"short\":\"12345678\",\"warnings\":[]}]" ""))
    (let (offered initial)
      (cl-letf (((symbol-function 'read-char-choice)
                 (lambda (_prompt chars) (setq offered chars) ?H))
                ((symbol-function 'samu--read-line)
                 (lambda (_prompt &optional start) (setq initial start) "Wauwau /Hund"))
                ((symbol-function 'samu--refresh-all) #'ignore))
        (with-temp-buffer (samu-capture-template)))
      (should (equal offered '(?h ?H)))
      (should (equal initial '(" /Hund" . 1)))
      (should (equal (car (samu-test--called '("add")))
                     '("add" "Wauwau /Hund" "--format" "json")))))
  (with-temp-buffer
    (samu-list-mode)
    (should (eq (key-binding (kbd "C")) #'samu-capture-template)))
  (with-temp-buffer
    (samu-task-mode)
    (should (eq (key-binding (kbd "C")) #'samu-comment))))

(ert-deftest samu-test-capture-template-needs-a-key ()
  (samu-test--with-cli '((("template" "list") 0 "[{\"call\":\"/inbox\",\"title\":\"inbox\"}]" ""))
    (should-error (samu--choose-template) :type 'user-error)))

;;;; Clock

(ert-deftest samu-test-clock-toggle-stops-only-its-own ()
  (samu-test--with-cli '((("clock" "status") 0 "[{\"owner\":\"x\",\"title\":\"Milch\"}]" "")
                         (("clock" "out") 0 "[{\"length\":\"0:05\",\"title\":\"Milch\"}]" ""))
    (with-temp-buffer
      (samu-list-mode)
      (cl-letf (((symbol-function 'samu--refresh-all) #'ignore))
        (setq samu--task-id "x")
        (samu-clock-toggle)
        (setq samu--task-id "y")
        (samu-clock-toggle)))
    (should (equal (samu-test--called '("clock" "out")) '(("clock" "out" "--format" "json"))))
    (should (equal (samu-test--called '("clock" "in")) '(("clock" "in" "y" "--format" "json"))))))

(ert-deftest samu-test-clock-text ()
  (let ((samu--clock `((start . ,(format-time-string "%FT%TZ" (time-subtract nil 3720) t))
                       (title . "Milch"))))
    (should (equal (substring-no-properties (samu--clock-text)) " ⏱ 1:02 Milch")))
  (let ((samu--clock nil)) (should (null (samu--clock-text)))))

;;;; Task's buffer

(ert-deftest samu-test-descendants ()
  (let ((all (list (samu-test--row "a" "A" :project "p")
                   (samu-test--row "b" "B" :project "p" :parent "a")
                   (samu-test--row "c" "C" :project "p" :parent "b")
                   (samu-test--row "d" "D" :project "q"))))
    (should (equal (mapcar (lambda (r) (alist-get 'id r)) (samu--descendants "a" "task" all))
                   '("b" "c")))
    (should (equal (mapcar (lambda (r) (alist-get 'id r)) (samu--descendants "p" "project" all))
                   '("a" "b" "c")))))

(ert-deftest samu-test-when-text ()
  (should (equal (samu--when-text '((date . "2026-11-12"))) "2026-11-12"))
  (should (null (samu--when-text nil)))
  (let ((process-environment (cons "TZ=UTC" process-environment)))
    (set-time-zone-rule "UTC")
    (unwind-protect
        (should (equal (samu--when-text '((at . "2026-11-12T08:30:00Z") (zone . "UTC")))
                       "2026-11-12 08:30"))
      (set-time-zone-rule nil))))

(ert-deftest samu-test-task-buffer-shows-fields-note-and-children ()
  (samu-test--with-cli
      `((("show") 0 "[{\"shown\":{\"id\":\"x\",\"kind\":\"task\",\"title\":\"Milch\",\"object\":{\"id\":\"x\",\"title\":\"Milch\",\"note\":\"eine Notiz\",\"priority\":\"A\",\"due\":{\"date\":\"2026-11-12\"}}},\"note_format\":\"markdown\",\"row\":{\"id\":\"x\",\"title\":\"Milch\",\"project\":\"Haushalt\",\"state\":\"Aufgabe\",\"available\":true,\"tags\":\"@agenda\",\"status\":{\"task\":{\"id\":\"x\"}}},\"below\":[{\"id\":\"k\",\"title\":\"Kind\",\"available\":true,\"status\":{\"task\":{\"id\":\"k\",\"parent\":\"x\"}}}],\"habits\":[],\"habit\":null,\"reminders\":[],\"props\":[],\"comments\":[{\"author\":\"test\",\"created\":\"2026-11-12T08:30:00Z\",\"body\":\"Hallo\",\"format\":\"markdown\"}],\"attachments\":[],\"log\":null}]" ""))
    (with-current-buffer (samu-show "x")
      (unwind-protect
          (let ((text (buffer-substring-no-properties (point-min) (point-max))))
            ;; one call fills it, and what the texts read as came with it
            (should (equal (mapcar #'car samu-test--calls)
                           '(("show" "x" "--full" "--format" "json"))))
            (should (string-match-p "\\`Milch\n" text))
            (should (string-match-p "^Where +#Haushalt$" text))
            (should (string-match-p "^State +Aufgabe$" text))
            (should (string-match-p "^Priority +A$" text))
            (should (string-match-p "^Due +2026-11-12$" text))
            (should (string-match-p "^Note\neine Notiz\n" text))
            (should (string-match-p "^Subtasks\n  Kind\n" text))
            (should (string-match-p "^Comments\ntest, .*\nHallo\n" text))
            (should (equal (samu--id-at-point) "x"))
            (goto-char (point-min))
            (search-forward "Kind")
            (should (equal (samu--id-at-point) "k")))
        (kill-buffer)))))

;;;; Keys

(ert-deftest samu-test-help-and-quit-are-bound-everywhere ()
  ;; Regression: `?' was undefined in a task's buffer.
  (dolist (map (list samu-list-mode-map samu-task-mode-map))
    (should (eq (lookup-key map "?") #'describe-mode))
    (should (eq (lookup-key map "q") #'quit-window))
    (should (eq (lookup-key map "g") #'samu-refresh)))
  (with-temp-buffer
    (samu-task-mode)
    (should (eq (key-binding "?") #'describe-mode))))

(ert-deftest samu-test-loading-again-binds-into-the-map-in-use ()
  ;; Regression: keys bound inside the `defvar' never reached an Emacs
  ;; that had loaded an older samu.el, since `defvar' keeps the value.
  (let ((map samu-list-mode-map))
    (define-key map "?" nil)
    (load (locate-library "samu.el") nil t t)
    (should (eq samu-list-mode-map map))
    (should (eq (lookup-key map "?") #'describe-mode))))

;;;; Following changes

(ert-deftest samu-test-note-format-is-asked-once-for-each-text ()
  (samu-test--with-cli '((("render") 0 "org\n" ""))
    (should (equal (samu--note-format "* a") "org"))
    (should (equal (samu--note-format "* a") "org"))
    (should (equal (samu--note-format "* b") "org"))
    (should (= (length (samu-test--called '("render"))) 2))
    (samu--forget)
    (samu--note-format "* a")
    (should (= (length (samu-test--called '("render"))) 3)))
  ;; a failure is not remembered
  (samu-test--with-cli '((("render") 1 "" "samu: no"))
    (should (equal (samu--note-format "x") "markdown"))
    (should (equal (samu--note-format "x") "markdown"))
    (should (= (length (samu-test--called '("render"))) 2))))

(ert-deftest samu-test-a-buffer-in-no-window-fills-once-it-is-shown ()
  (let ((rows (list (samu-test--row "t1" "Milch")))
        (seen (generate-new-buffer "*samu-test-seen*"))
        (buried (generate-new-buffer "*samu-test-buried*"))
        (fetched nil))
    (unwind-protect
        (samu-test--with-cli nil
          (dolist (buffer (list seen buried))
            (with-current-buffer buffer
              (samu-list-mode)
              (setq samu--view (list :title "T"
                                     :fetch (lambda ()
                                              (push (buffer-name) fetched)
                                              (list (cons nil rows)))))
              (samu--render)))
          (setq fetched nil
                rows (list (samu-test--row "t2" "Brot")))
          ;; from a third buffer: the one in a window fills, the other waits
          (set-window-buffer (selected-window) seen)
          (with-temp-buffer (samu--refresh-all))
          (should (equal fetched '("*samu-test-seen*")))
          (should (string-match-p "Brot" (with-current-buffer seen (buffer-string))))
          (should (string-match-p "Milch" (with-current-buffer buried (buffer-string))))
          (should (buffer-local-value 'samu--stale buried))
          (should-not (buffer-local-value 'samu--stale seen))
          ;; shown, it fills, once
          (should (memq #'samu--shown
                        (buffer-local-value 'window-buffer-change-functions buried)))
          (set-window-buffer (selected-window) buried)
          (samu--shown (selected-window))
          (samu--shown (selected-window))
          (should (equal fetched '("*samu-test-buried*" "*samu-test-seen*")))
          (should (string-match-p "Brot" (with-current-buffer buried (buffer-string))))
          (should-not (buffer-local-value 'samu--stale buried)))
      (kill-buffer seen)
      (kill-buffer buried))))

(ert-deftest samu-test-the-buffer-acted-in-fills-also-in-no-window ()
  (let ((fetched 0))
    (samu-test--with-cli nil
      (with-temp-buffer
        (samu-list-mode)
        (setq samu--view (list :title "T" :fetch (lambda () (cl-incf fetched) nil)))
        (should-not (get-buffer-window (current-buffer) t))
        (samu--after-action nil)
        (should (= fetched 1))
        (should-not samu--stale)))))

;;;; Asking without waiting

;; These tests run a real process: a shell script in place of the CLI
;; that notes when each call starts and ends, sleeps and prints
;; synthetic JSON.

(defvar samu-test--log nil
  "The file the script in place of the CLI notes its calls in.")

(defconst samu-test--script
  "sleep 0.2
case \"$1\" in
  show) echo '[{\"shown\":{\"kind\":\"task\",\"title\":\"Milch\",\"object\":{\"id\":\"t1\",\"title\":\"Milch\",\"note\":\"eine Notiz\"}},\"note_format\":\"markdown\",\"row\":{\"id\":\"t1\",\"title\":\"Milch\",\"available\":true,\"project\":\"\",\"status\":{\"task\":{\"id\":\"t1\",\"title\":\"Milch\"}}},\"below\":[],\"comments\":[{\"author\":\"wer\",\"created\":\"2026-11-01T10:00:00Z\",\"body\":\"ein Kommentar\",\"format\":\"markdown\"}]}]' ;;
  list) echo '[{\"id\":\"t1\",\"short\":\"t1\",\"title\":\"Milch\",\"available\":true,\"project\":\"\",\"status\":{\"task\":{\"id\":\"t1\",\"title\":\"Milch\"}}}]' ;;
  render) cat >/dev/null; echo markdown ;;
  *) echo '[]' ;;
esac
"
  "What the script in place of the CLI does by default.")

(defmacro samu-test--with-script (script &rest body)
  "Run BODY with a shell script in place of the CLI, without `samu--sync'.
SCRIPT is what it does after noting \"+ ARGS\"; it notes \"- ARGS\" at its end."
  (declare (indent 1))
  `(let* ((dir (make-temp-file "samu-test" t))
          (samu-executable (expand-file-name "samu" dir))
          (samu-test--log (expand-file-name "log" dir))
          (samu--sync nil)
          (samu--cache nil)
          (samu--note-formats (make-hash-table :test #'equal))
          (samu-profile nil) (samu-server nil) (samu-account nil) (samu-scope nil))
     (unwind-protect
         (progn (samu-test--write-script ,script)
                ,@body)
       (samu-test--wait (lambda () (zerop samu--running)))
       (delete-directory dir t))))

(defun samu-test--write-script (script)
  "Make SCRIPT what the script in place of the CLI does."
  (with-temp-file samu-executable
    (insert "#!/bin/sh\necho \"+ $*\" >> " samu-test--log "\n"
            "trap 'echo \"- $*\" >> " samu-test--log "' EXIT\n"
            script))
  (set-file-modes samu-executable #o700))

(defun samu-test--wait (done)
  "Let processes and timers run until DONE returns non-nil, at most 10 s."
  (let ((end (+ (float-time) 10)))
    (while (and (not (funcall done)) (< (float-time) end))
      (accept-process-output nil 0.02))
    (funcall done)))

(defun samu-test--log-lines ()
  "The lines the script noted, oldest first."
  (and (file-exists-p samu-test--log)
       (with-temp-buffer
         (insert-file-contents samu-test--log)
         (split-string (buffer-string) "\n" t))))

(defun samu-test--started (command)
  "How many calls of COMMAND the script noted."
  (cl-count-if (lambda (line) (string-prefix-p (concat "+ " command) line))
               (samu-test--log-lines)))

(defun samu-test--most-at-once ()
  "The most calls of the script that ran at the same time."
  (let ((now 0) (most 0))
    (dolist (line (samu-test--log-lines))
      (setq now (if (string-prefix-p "+" line) (1+ now) (1- now))
            most (max most now)))
    most))

(ert-deftest samu-test-run-async-gives-code-stdout-and-stderr ()
  (samu-test--with-script "cat; echo 'samu: nein' >&2; exit 3\n"
    (let ((answer nil))
      (samu--run-async '("comment" "t1" "-") "Größe" (lambda (a) (setq answer a)))
      (should (null answer))
      (should (samu-test--wait (lambda () answer)))
      (should (equal answer '(3 "Größe" "samu: nein")))
      (should (equal (samu-test--log-lines) '("+ comment t1 -" "- comment t1 -"))))))

(ert-deftest samu-test-run-async-without-the-cli-answers-with-an-error ()
  (samu-test--with-script ""
    (let ((samu-executable (concat samu-executable "-missing")) (answer nil))
      (samu--run-async '("list") nil (lambda (a) (setq answer a)))
      (should (null answer))
      (should (samu-test--wait (lambda () answer)))
      (should (= (car answer) 127))
      (should (zerop samu--running)))))

(ert-deftest samu-test-run-async-runs-six-at-once-and-no-more ()
  (samu-test--with-script "sleep 0.1\n"
    (let ((answers 0))
      (dotimes (i 14)
        (samu--run-async (list "show" (number-to-string i)) nil
                         (lambda (_) (cl-incf answers))))
      (should (= samu--running 6))
      (should (samu-test--wait (lambda () (= answers 14))))
      (should (= (samu-test--most-at-once) 6))
      (should (zerop samu--running))
      (should (null samu--waiting)))))

(ert-deftest samu-test-a-task-buffer-fills-without-emacs-waiting ()
  (samu-test--with-script samu-test--script
    (let ((buffer (samu-show "t1")))
      (unwind-protect
          (with-current-buffer buffer
            ;; `samu-show' is back before the first call ended
            (should (string-match-p "Loading" (buffer-string)))
            (should-not (cl-some (lambda (line) (string-prefix-p "-" line))
                                 (samu-test--log-lines)))
            (should samu--fetching)
            (should (samu-test--wait (lambda () (not samu--fetching))))
            (let ((text (buffer-string)))
              (should (string-match-p "\\`Milch" text))
              (should (string-match-p "eine Notiz" text))
              (should (string-match-p "ein Kommentar" text))
              (should-not (string-match-p "Loading" text)))
            (should (equal (buffer-name) "*samu: Milch*"))
            ;; one call for all of it
            (should (equal (samu-test--log-lines)
                           '("+ show t1 --full --format json"
                             "- show t1 --full --format json")))
            ;; drawing asked nothing more
            (should (zerop samu--running)))
        (kill-buffer buffer)))))

(ert-deftest samu-test-asked-again-while-filling-fills-once-more ()
  (samu-test--with-script samu-test--script
    (with-temp-buffer
      (samu-list-mode)
      (setq samu--view (list :title "T" :fetch (lambda () (list (cons nil (samu--list))))))
      (samu--render)
      (samu--render)
      (samu--render)
      (should (eq samu--fetching 'again))
      (should (samu-test--wait (lambda () (not samu--fetching))))
      (should (= (samu-test--started "list") 2))
      (should (string-match-p "Milch" (buffer-string))))))

(ert-deftest samu-test-a-buffer-killed-while-filling-is-left-alone ()
  (samu-test--with-script samu-test--script
    (let ((messages nil))
      (cl-letf (((symbol-function 'message)
                 (lambda (&rest args) (push (apply #'format args) messages))))
        (kill-buffer (samu-show "t1"))
        (should (samu-test--wait (lambda () (and (zerop samu--running)
                                                 (null samu--waiting))))))
      ;; it stopped after the call that was running: nothing more was asked
      (should (= (length (samu-test--log-lines)) 2)))))

(ert-deftest samu-test-a-failing-refill-keeps-what-the-buffer-shows ()
  (samu-test--with-script samu-test--script
    (with-temp-buffer
      (samu-list-mode)
      (setq samu--view (list :title "T" :fetch (lambda () (list (cons nil (samu--list))))))
      (samu--render)
      (should (samu-test--wait (lambda () (not samu--fetching))))
      (let ((before (buffer-string)) (messages nil))
        (should (string-match-p "Milch" before))
        (samu-test--write-script "echo 'samu: kein Netz' >&2; exit 1\n")
        (cl-letf (((symbol-function 'message)
                   (lambda (&rest args) (push (apply #'format args) messages))))
          (samu--render)
          (should (samu-test--wait (lambda () (not samu--fetching)))))
        (should (equal (buffer-string) before))
        (should (equal messages '("samu: kein Netz")))
        (should (null mode-line-process))))))

(ert-deftest samu-test-a-buffer-never-filled-shows-the-error ()
  (samu-test--with-script "echo 'samu: kein Netz' >&2; exit 1\n"
    (with-temp-buffer
      (samu-list-mode)
      (setq samu--view (list :title "T" :fetch (lambda () (list (cons nil (samu--list))))))
      (cl-letf (((symbol-function 'message) #'ignore))
        (samu--render)
        (should (string-match-p "Loading" (buffer-string)))
        (should (samu-test--wait (lambda () (not samu--fetching)))))
      (should (string-match-p "\\`samu: kein Netz" (buffer-string)))
      ;; and `g' fills it once the CLI answers
      (samu-test--write-script samu-test--script)
      (samu-refresh)
      (should (samu-test--wait (lambda () (not samu--fetching))))
      (should (string-match-p "Milch" (buffer-string)))
      (should-not (string-match-p "kein Netz" (buffer-string))))))

(ert-deftest samu-test-the-clock-is-asked-without-waiting ()
  (samu-test--with-script
      "sleep 0.1; echo '[{\"owner\":\"t1\",\"title\":\"Milch\",\"start\":\"2026-11-01T10:00:00Z\"}]'\n"
    (let ((samu-clock-mode-line-mode t) (samu--clock nil))
      (samu--clock-update)
      (should (null samu--clock))
      (should (samu-test--wait (lambda () samu--clock)))
      (should (equal (samu--get samu--clock 'title) "Milch")))))

(ert-deftest samu-test-live-command ()
  (let ((samu-executable "samu") (samu-server "http://127.0.0.1:1") (samu-account nil))
    (should (equal (samu--live-command)
                   '("samu" "--server" "http://127.0.0.1:1" "events" "--format" "jsonl")))))

(ert-deftest samu-test-live-is-wanted-only-with-a-buffer-or-the-clock ()
  (let ((samu-live t) (samu-clock-mode-line-mode nil))
    (cl-letf (((symbol-function 'samu--live-buffers) (lambda () nil)))
      (should-not (samu--live-wanted-p))
      (let ((samu-clock-mode-line-mode t)) (should (samu--live-wanted-p))))
    (cl-letf (((symbol-function 'samu--live-buffers) (lambda () '(x))))
      (should (samu--live-wanted-p))
      (let ((samu-live nil)) (should-not (samu--live-wanted-p))))))

(ert-deftest samu-test-live-events-refresh-once-after-a-quiet-moment ()
  (let ((samu--live-timer nil) (refreshed 0) (scheduled 0))
    (cl-letf (((symbol-function 'run-at-time)
               (lambda (&rest _) (cl-incf scheduled) (timer-create)))
              ((symbol-function 'samu--refresh-all) (lambda () (cl-incf refreshed)))
              ((symbol-function 'samu--clock-update) #'ignore)
              ((symbol-function 'active-minibuffer-window) (lambda () nil)))
      (samu--live-filter nil "{\"event\":\"changed\"}\n{\"event\":")
      (samu--live-filter nil "\"changed\"}\n")
      (samu--live-filter nil "no line yet")
      (should (= scheduled 2))
      (should (= refreshed 0))
      (samu--live-refresh)
      (should (= refreshed 1))
      (should (null samu--live-timer)))))

(ert-deftest samu-test-live-waits-while-something-is-read ()
  (let ((samu--live-timer nil) (refreshed 0))
    (cl-letf (((symbol-function 'run-at-time) (lambda (&rest _) (timer-create)))
              ((symbol-function 'samu--refresh-all) (lambda () (cl-incf refreshed)))
              ((symbol-function 'active-minibuffer-window) (lambda () t)))
      (samu--live-refresh)
      (should (= refreshed 0))
      (should samu--live-timer)
      (setq samu--live-timer nil))))

(ert-deftest samu-test-live-connects-again-later-and-later ()
  (let ((samu-live t) (samu--live-retry 5) (samu--live-retry-timer nil) (delays nil)
        (process (make-process :name "samu-test-dead" :command '("true") :noquery t)))
    (while (process-live-p process) (accept-process-output process 0.05))
    (cl-letf (((symbol-function 'samu--live-wanted-p) (lambda () t))
              ((symbol-function 'run-at-time)
               (lambda (delay &rest _) (push delay delays) (timer-create))))
      (dotimes (_ 6)
        (setq samu--live-process process samu--live-retry-timer nil)
        (samu--live-sentinel process "finished\n")
        (should (null samu--live-process))))
    (should (equal (reverse delays) '(5 10 20 40 60 60)))
    (samu--live-filter nil "\n")
    (should (= samu--live-retry 5))
    (cancel-function-timers #'samu--live-refresh)
    (setq samu--live-timer nil)))

(ert-deftest samu-test-live-ends-with-the-last-buffer ()
  (let ((samu-live t) (stopped nil))
    (cl-letf (((symbol-function 'samu--live-buffers) (lambda () nil))
              ((symbol-function 'samu--live-stop) (lambda () (setq stopped t)))
              ((symbol-function 'samu--live-start) (lambda () (error "started"))))
      (samu--live-ensure)
      (should stopped))))

(ert-deftest samu-test-refresh-keeps-each-window-where-it-was ()
  (let ((rows (cl-loop for i below 200
                       collect (samu-test--row (format "id%03d" i) (format "Aufgabe %d" i)))))
    (with-current-buffer (get-buffer-create "*samu-test-window*")
      (unwind-protect
          (progn
            (samu-list-mode)
            (setq samu--view (list :title "T" :fetch (lambda () (list (cons nil rows)))))
            (samu--render)
            (let ((window (selected-window)))
              (set-window-buffer window (current-buffer))
              (goto-char (point-min))
              (search-forward "Aufgabe 150")
              (set-window-start window (line-beginning-position -5))
              (should (equal (samu--id-from (window-start window)) "id144"))
              (setq rows (cons (samu-test--row "neu" "Neu") rows))
              (with-temp-buffer (with-current-buffer "*samu-test-window*" (samu--render)))
              (should (equal (get-text-property (window-point window) 'samu-id) "id150"))
              (should (equal (get-text-property (window-start window) 'samu-id) "id144"))))
        (kill-buffer)))))

;;;; Habits (ADR 0091 §6)

(defun samu-test--habit (id title habit &rest status)
  "A record of `samu habit list' for ID with TITLE, HABIT and STATUS fields.
STATUS is a plist: :day, :amount, :target, :met, :streak, :longest."
  (let ((streak (or (plist-get status :streak) 0)))
    `((id . ,id) (short . ,id) (title . ,title) (streak . ,streak)
      (longest . ,(or (plist-get status :longest) streak))
      (recent . "3/4") (year . "0/0") (last_fail . ,(plist-get status :last-fail))
      (status . ((task . ((id . ,id) (title . ,title) (habit . ,habit)))
                 (today . "2026-11-13")
                 (day . ,(or (plist-get status :day) "none"))
                 (period . ((amount . ,(or (plist-get status :amount) 0))
                            (target . ,(or (plist-get status :target) 1))
                            (met . ,(plist-get status :met))))
                 (streak . ,streak))))))

(defun samu-test--json (value)
  "VALUE, alists and lists, as JSON text."
  (let ((json-encoding-pretty-print nil)) (json-encode value)))

(ert-deftest samu-test-habit-goal ()
  (should (equal (samu--habit-goal '((kind . "build") (times . 1) (per . "day"))) "daily"))
  (should (equal (samu--habit-goal '((kind . "build") (times . 3) (per . "week"))) "3× a week"))
  (should (equal (samu--habit-goal '((kind . "build") (times . 8) (per . "day") (unit . "Glas")))
                 "8 Glas a day"))
  (should (equal (samu--habit-goal '((kind . "avoid") (times . 1) (per . "day"))) "avoid")))

(ert-deftest samu-test-habit-cell ()
  (let ((samu-habit-streak-mark "*"))
    (should (equal (substring-no-properties
                    (samu--habit-cell (samu-test--habit "d" "Dehnen" '((kind . "build") (times . 1) (per . "day"))
                                                        :day "done" :amount 1 :met t :streak 4)))
                   "✓ *4"))
    (should (equal (substring-no-properties
                    (samu--habit-cell (samu-test--habit "w" "Wasser"
                                                        '((kind . "build") (times . 8) (per . "day") (unit . "Glas"))
                                                        :day "partial" :amount 5 :target 8 :streak 2)))
                   "○ 5/8 Glas *2"))
    (should (equal (substring-no-properties
                    (samu--habit-cell (samu-test--habit "k" "Kraft" '((kind . "build") (times . 3) (per . "week"))
                                                        :amount 2 :target 3)))
                   "○ 2/3 week"))
    (let ((failed (samu--habit-cell (samu-test--habit "d" "Dehnen" '((kind . "build")) :day "fail"))))
      (should (equal (substring-no-properties failed) "✗"))
      (should (eq (get-text-property 0 'face failed) 'samu-habit-fail)))
    (should (equal (substring-no-properties
                    (samu--habit-cell (samu-test--habit "r" "Rauchfrei" '((kind . "avoid"))
                                                        :day "done" :met t :streak 12)))
                   "free for 12 d"))))

(ert-deftest samu-test-a-view-shows-habits-with-one-call ()
  (let ((rows (list (samu-test--row "d" "Dehnen" :habit '((kind . "build") (times . 1) (per . "day")))
                    (samu-test--row "m" "Milch")
                    (samu-test--row "w" "Wasser" :habit '((kind . "build") (times . 8) (unit . "Glas")))))
        (samu-habit-streak-mark "*"))
    (samu-test--with-cli
        `((("habit" "list") 0
           ,(samu-test--json
             (list (samu-test--habit "d" "Dehnen" '((kind . "build") (times . 1) (per . "day"))
                                     :day "done" :amount 1 :met t :streak 4)
                   (samu-test--habit "w" "Wasser" '((kind . "build") (times . 8) (unit . "Glas"))
                                     :day "partial" :amount 5 :target 8)))
           ""))
      (with-temp-buffer
        (samu-list-mode)
        (setq samu-scope "Habit"
              samu--view (list :title "T" :fetch (lambda () (list (cons nil rows)))))
        (samu--render)
        (should (equal (samu-test--called '("habit"))
                       '(("habit" "list" "--scope" "Habit" "--format" "json"))))
        (should (equal (buffer-substring-no-properties (point-min) (point-max))
                       "T  Scope Habit\n  ✓ *4 Dehnen\n  Milch\n  ○ 5/8 Glas Wasser\n"))))
    ;; Without a habit among the rows, no call.
    (samu-test--with-cli nil
      (with-temp-buffer
        (samu-list-mode)
        (setq samu--view (list :title "T" :fetch (lambda () (list (cons nil (list (cadr rows)))))))
        (samu--render)
        (should (null (samu-test--called '("habit"))))))))

(ert-deftest samu-test-habit-done-fail-pause ()
  (let ((answer (samu-test--json (list (samu-test--habit "d" "Dehnen" '((kind . "build"))
                                                          :day "done" :met t)))))
    (samu-test--with-cli `((("habit") 0 ,answer ""))
      (with-temp-buffer
        (samu-list-mode)
        (setq samu--task-id "d"
              samu--task-row (samu-test--row "d" "Dehnen" :habit '((kind . "build"))))
        (cl-letf (((symbol-function 'samu--refresh-all) #'ignore)
                  ((symbol-function 'samu--read-date) (lambda (&rest _) "2026-11-10")))
          (samu-habit-done nil)
          (samu-habit-done 3)
          (samu-habit-done '(4))
          (samu-habit-fail nil)
          (samu-habit-fail '(4))
          (samu-habit-pause "2026-11-13" "2026-11-13")
          (samu-habit-pause "2026-11-14" "2026-11-20")
          (should-error (samu-habit-done 0) :type 'user-error))
        (should (equal (samu-test--called '("habit"))
                       '(("habit" "done" "d" "--format" "json")
                         ("habit" "done" "d" "--amount" "3" "--format" "json")
                         ("habit" "done" "d" "--day" "2026-11-10" "--format" "json")
                         ("habit" "fail" "d" "--format" "json")
                         ("habit" "fail" "d" "--day" "2026-11-10" "--format" "json")
                         ("habit" "pause" "d" "--from" "2026-11-13" "--format" "json")
                         ("habit" "pause" "d" "--from" "2026-11-14" "--until" "2026-11-20"
                          "--format" "json"))))))))

(ert-deftest samu-test-habit-keys-refuse-a-plain-task ()
  (samu-test--with-cli nil
    (with-temp-buffer
      (samu-list-mode)
      (setq samu--task-id "m" samu--task-row (samu-test--row "m" "Milch"))
      (should-error (samu-habit-done nil) :type 'user-error)
      (should (null samu-test--calls)))))

(ert-deftest samu-test-habit-undo-takes-the-entry-made-last ()
  (samu-test--with-cli
      '((("habit" "entries") 0
         "[{\"id\":\"e2\",\"day\":\"2026-11-13\",\"status\":\"done\",\"amount\":1,\"entry\":{\"created\":\"2026-11-13T07:00:00Z\"}},
           {\"id\":\"e3\",\"day\":\"2026-11-12\",\"status\":\"done\",\"amount\":2,\"entry\":{\"created\":\"2026-11-13T09:00:00Z\"}},
           {\"id\":\"e1\",\"day\":\"2026-11-12\",\"status\":\"fail\",\"entry\":{\"created\":\"2026-11-12T20:00:00Z\"}}]" "")
        (("habit" "undo") 0 "" ""))
    (with-temp-buffer
      (samu-list-mode)
      (setq samu--task-id "d" samu--task-row (samu-test--row "d" "Dehnen" :habit '((kind . "build"))))
      (let ((asked nil))
        (cl-letf (((symbol-function 'samu--refresh-all) #'ignore)
                  ((symbol-function 'y-or-n-p) (lambda (prompt) (setq asked prompt) t)))
          (samu-habit-undo))
        (should (equal asked "Delete the entry 2026-11-12  done 2? ")))
      (should (equal (samu-test--called '("habit" "undo"))
                     '(("habit" "undo" "e3" "--format" "json")))))))

(ert-deftest samu-test-habit-weeks-start-on-monday ()
  ;; 2026-11-11 is a Wednesday, 2026-11-16 a Monday.
  (let ((weeks (samu--habit-weeks
                (mapcar (lambda (d) `((day . ,d) (state . "done")))
                        '("2026-11-11" "2026-11-12" "2026-11-13" "2026-11-14" "2026-11-15"
                          "2026-11-16" "2026-11-17")))))
    (should (= (length weeks) 2))
    (should (equal (mapcar (lambda (d) (alist-get 'day d)) (car weeks))
                   '(nil nil "2026-11-11" "2026-11-12" "2026-11-13" "2026-11-14" "2026-11-15")))
    (should (equal (mapcar (lambda (d) (alist-get 'day d)) (cadr weeks))
                   '("2026-11-16" "2026-11-17")))))

(ert-deftest samu-test-habit-rate ()
  (should (equal (samu--habit-rate "3/4") "75 % (3/4)"))
  (should (equal (samu--habit-rate "0/0") "–")))

(ert-deftest samu-test-habit-buffer-shows-facts-calendar-and-entries ()
  (let* ((record (samu-test--habit "w" "Wasser" '((kind . "build") (times . 8) (unit . "Glas"))
                                   :day "partial" :amount 5 :target 8 :streak 2 :longest 9))
         (status (alist-get 'status record)))
    (setf (alist-get 'status record)
          (append status
                  '((history . (((day . "2026-11-11") (state . "done") (amount . 8))
                                ((day . "2026-11-12") (state . "fail") (amount . 0))
                                ((day . "2026-11-13") (state . "partial") (amount . 5))))
                    (entries . (((id . "e1") (day . "2026-11-13") (status . "done")
                                 (created . "2026-11-13T07:00:00Z")
                                 (created_by . ((device . "phone"))))
                                ((id . "e0") (day . "2026-11-12") (status . "fail")
                                 (created . "2026-11-12T21:00:00Z")))))))
    (samu-test--with-cli `((("habit" "show") 0 ,(samu-test--json (list record)) "")
                           (("habit" "undo") 0 "" ""))
      (let ((samu-habit-streak-mark "*"))
        (cl-letf (((symbol-function 'samu--live-ensure) #'ignore)
                  ((symbol-function 'samu--habit-history-days) (lambda () 3)))
          (with-current-buffer (samu-habit-show "w")
            (unwind-protect
                (let ((text (buffer-substring-no-properties (point-min) (point-max))))
                  (should (equal (car (samu-test--called '("habit" "show")))
                                 '("habit" "show" "w" "--days" "3" "--format" "json")))
                  (should (string-match-p "\\`Wasser\n" text))
                  (should (string-match-p "^Goal +8 Glas a day$" text))
                  (should (string-match-p "^Today +○ 5/8 Glas \\*2$" text))
                  (should (string-match-p "^Streak +2, longest 9$" text))
                  (should (string-match-p "^Rate +4 weeks 75 % (3/4), year –$" text))
                  (should (string-match-p "^  11-09        #  x  \\+$" text))
                  (should (string-match-p "^Entries\n  2026-11-13  done  phone\n  2026-11-12  not done\n"
                                          text))
                  (should (equal (samu--id-at-point) "w"))
                  (goto-char (point-min))
                  (search-forward "2026-11-12  not")
                  (cl-letf (((symbol-function 'y-or-n-p) (lambda (_) t))
                            ((symbol-function 'samu--refresh-all) #'ignore))
                    (call-interactively (key-binding (kbd "DEL"))))
                  (should (equal (samu-test--called '("habit" "undo"))
                                 '(("habit" "undo" "e0" "--format" "json")))))
              (kill-buffer))))))))

(ert-deftest samu-test-task-buffer-of-a-habit-shows-its-streak ()
  (samu-test--with-cli
      `((("show") 0
         ,(samu-test--json
           `(((shown . ((id . "d") (kind . "task") (title . "Dehnen")
                        (object . ((id . "d") (title . "Dehnen")
                                   (habit . ((kind . "build") (times . 1) (per . "day")))))))
              (habit . ,(samu-test--habit "d" "Dehnen" '((kind . "build") (times . 1) (per . "day"))
                                          :day "done" :met t :streak 4 :longest 7)))))
         ""))
    (let ((samu-habit-streak-mark "*"))
      (cl-letf (((symbol-function 'samu--live-ensure) #'ignore))
        (with-current-buffer (samu-show "d")
          (unwind-protect
              (let ((text (buffer-substring-no-properties (point-min) (point-max))))
                (should (equal (mapcar #'car samu-test--calls)
                               '(("show" "d" "--full" "--format" "json"))))
                (should (string-match-p "^Goal +daily$" text))
                (should (string-match-p "^Today +✓ \\*4$" text))
                (should (string-match-p "^Streak +4, longest 7$" text)))
            (kill-buffer)))))))

(ert-deftest samu-test-habit-keys ()
  (dolist (map (list samu-list-mode-map samu-task-mode-map samu-habit-mode-map))
    (should (eq (lookup-key map (kbd "h h")) #'samu-habit-done))
    (should (eq (lookup-key map (kbd "h d")) #'samu-habit-done))
    (should (eq (lookup-key map (kbd "h f")) #'samu-habit-fail))
    (should (eq (lookup-key map (kbd "h p")) #'samu-habit-pause))
    (should (eq (lookup-key map (kbd "h u")) #'samu-habit-undo))
    (should (eq (lookup-key map (kbd "h s")) #'samu-habit-show))))

;;;; Links

(ert-deftest samu-test-link-ids ()
  (should (equal (samu--id "01A1CF7D-0B40-7799-9A36-D0B256839F42")
                 "01a1cf7d-0b40-7799-9a36-d0b256839f42"))
  (should (equal (samu--id "//01a1cf7d-0b40-7799-9a36-d0b256839f42/")
                 "01a1cf7d-0b40-7799-9a36-d0b256839f42"))
  (should (null (samu--id "kein-link"))))

(ert-deftest samu-test-link-target ()
  (let ((samu-link-target 'emacs))
    (should-not (samu--link-in-browser-p nil))
    (should (samu--link-in-browser-p '(4))))
  (let ((samu-link-target 'browser))
    (should (samu--link-in-browser-p nil))
    (should-not (samu--link-in-browser-p '(4)))))

(ert-deftest samu-test-follow-link-opens-here ()
  (samu-test--with-cli '((("show") 0 "[{\"kind\":\"tag\"}]" ""))
    (let ((samu-link-target 'emacs) (opened nil))
      (cl-letf (((symbol-function 'samu--show-object)
                 (lambda (id kind) (setq opened (list id kind)))))
        (samu-follow-link "01a1cf7d-0b40-7799-9a36-d0b256839f42"))
      (should (equal opened '("01a1cf7d-0b40-7799-9a36-d0b256839f42" "tag"))))))

(ert-deftest samu-test-export-link ()
  (let ((samu-web-url "https://samu.example/"))
    (should (equal (samu-export-link "01a1cf7d-0b40-7799-9a36-d0b256839f42" "Milch" 'md)
                   "[Milch](https://samu.example/o/01a1cf7d-0b40-7799-9a36-d0b256839f42)"))
    (should (null (samu-export-link "x" nil 'odt))))
  (let ((samu-web-url nil) (samu-profile nil))
    (should (equal (samu-export-link "01a1cf7d-0b40-7799-9a36-d0b256839f42" "Milch" 'md)
                   "[Milch](samu:01a1cf7d-0b40-7799-9a36-d0b256839f42)"))))

(ert-deftest samu-test-url-of-a-profile-comes-from-the-cli ()
  (samu-test--with-cli
      '((("open-uri") 0 "[{\"id\":\"x\",\"command\":[\"xdg-open\",\"http://dienst.samu.localhost/o/x\"]}]" ""))
    (let ((samu-profile "dienst") (samu-web-url "https://samu.example"))
      (should (equal (samu--url "x") "http://dienst.samu.localhost/o/x"))
      (should (equal (car (car samu-test--calls))
                     '("open-uri" "samu:x" "--print" "--format" "json")))))
  (let ((samu-profile nil) (samu-web-url "https://samu.example/"))
    (should (equal (samu--url "x") "https://samu.example/o/x")))
  ;; Without either there is no address to guess.
  (let ((samu-profile nil) (samu-web-url nil))
    (should-error (samu--url "x") :type 'user-error)))

(ert-deftest samu-test-store-link ()
  (require 'ol)
  (with-temp-buffer
    (samu-list-mode)
    (let ((org-store-link-plist nil) (inhibit-read-only t))
      (samu--insert-rows (list (samu-test--row "x" "Milch")))
      (goto-char (point-min))
      (should (samu-store-link))
      (should (equal (plist-get org-store-link-plist :link) "samu:x"))
      (should (equal (plist-get org-store-link-plist :description) "Milch")))))

(provide 'test-samu)
;;; test-samu.el ends here

;; ADR 0088 §8: the view Review lists what is due, through the scope, and
;; `R' on a project row marks it reviewed; on a task row its project.
(ert-deftest samu-test-review ()
  (samu-test--with-cli
      '((("projects" "--review") 0
         "[{\"id\":\"p1\",\"title\":\"Keller\",\"available\":true,\"status\":\"active\",\"stuck\":\"\",\"review\":\"2026-11-01\",\"review_due\":\"due\",\"reasons\":\"\",\"project\":{\"project\":{\"id\":\"p1\"}}}]" "")
        (("review") 0 "" ""))
    (cl-letf (((symbol-function 'samu--refresh-all) #'ignore)
              ((symbol-function 'samu--live-ensure) #'ignore))
      (with-current-buffer (samu-review)
        (should (equal (car (samu-test--called '("projects")))
                       '("projects" "--review" "--format" "json")))
        (goto-char (point-min))
        (should (search-forward "review due since 2026-11-01" nil t))
        (goto-char (point-min))
        (search-forward "Keller")
        (samu-mark-reviewed)
        (should (equal (car (samu-test--called '("review"))) '("review" "p1")))
        (kill-buffer)))
    (with-temp-buffer
      (insert (propertize "Regal" 'samu-row (samu-test--row "t1" "Regal" :project "p2")))
      (goto-char (point-min))
      (cl-letf (((symbol-function 'samu--refresh-all) #'ignore))
        (samu-mark-reviewed))
      (should (equal (car (last (samu-test--called '("review")))) '("review" "p2"))))))

(ert-deftest samu-test-an-empty-answer-is-remembered ()
  (samu-test--with-cli '((("template" "list") 0 "[]" ""))
    (should (null (samu--templates)))
    (should (null (samu--templates)))
    (should (= (length (samu-test--called '("template" "list"))) 1))))

(ert-deftest samu-test-a-missing-command-says-what-to-set ()
  (let* ((samu-executable "/nonexistent/samu")
         (samu--answers nil)
         (err (should-error (samu--call-text '("session")) :type 'samu-error)))
    (should (equal (error-message-string err)
                   (concat "samu: no command `/nonexistent/samu'; "
                           "install the CLI or set `samu-executable'")))))

(ert-deftest samu-test-a-missing-command-in-the-background ()
  (let ((samu-executable "/nonexistent/samu") (answer nil))
    (samu--spawn (list samu-executable "session") nil
                 (lambda (given) (setq answer given)))
    (should (samu-test--wait (lambda () answer)))
    (should (equal (car answer) 127))
    (should (equal (nth 2 answer) (samu--no-executable)))))

(ert-deftest samu-test-faces-need-no-org ()
  (should-not (featurep 'org))
  (dolist (face '(samu-section samu-state samu-priority samu-done samu-title))
    (should (facep (face-attribute face :inherit)))))

;;;; For other packages

(ert-deftest samu-test-tasks-go-through-the-scope ()
  (samu-test--with-cli '((("list") 0 "[{\"id\":\"t1\",\"status\":{\"task\":{\"due\":\"2026-11-20\"}}}]" ""))
    (let* ((samu-scope "Work")
           (rows (samu-tasks "--available")))
      (should (equal (samu-get (car rows) 'status 'task 'due) "2026-11-20"))
      (should (equal (car (samu-test--called '("list")))
                     '("list" "--available" "--scope" "Work" "--format" "json"))))))

(ert-deftest samu-test-task-is-nil-when-there-is-none ()
  (samu-test--with-cli '((("show" "t1") 0 "[{\"id\":\"t1\",\"kind\":\"task\"}]" "")
                         (("show" "nope") 3 "" "samu: no such object"))
    (should (equal (samu-get (samu-task "t1") 'kind) "task"))
    (should (null (samu-task "nope")))))

(ert-deftest samu-test-add-returns-the-record-and-tells-the-hooks ()
  (samu-test--with-cli '((("add") 0 "[{\"id\":\"n1\",\"title\":\"Milk @home\"}]" ""))
    (let* ((captured nil) (acted 0)
           (samu-after-capture-functions (list (lambda (record) (push record captured))))
           (samu-after-action-hook (list (lambda () (cl-incf acted))))
           (record (samu-add "Milk @home" :note "why" :literal t)))
      (should (equal (samu-get record 'id) "n1"))
      (should (equal captured (list record)))
      (should (= acted 1))
      (should (equal (car (samu-test--called '("add")))
                     '("add" "--note" "why" "--literal" "Milk @home"
                       "--format" "json"))))))

(ert-deftest samu-test-an-action-runs-its-hook-a-refresh-its-own ()
  (samu-test--with-cli '((("list") 0 "[]" ""))
    (let* ((acted 0) (filled nil)
           (samu-after-action-hook (list (lambda () (cl-incf acted))))
           (samu-after-refresh-hook (list (lambda () (push (buffer-name) filled)))))
      (with-temp-buffer
        (samu-list-mode)
        (setq samu--view (list :title "T" :fetch (lambda () (list (cons nil (samu--list))))))
        (samu--render)
        (should (equal filled (list (buffer-name))))
        (should (= acted 0))
        (samu--after-action nil)
        (should (= acted 1))
        (should (= (length filled) 2))))))

;;;; In the Org agenda

(defconst samu-test--forecast
  "[{\"section_kind\":\"overdue\",\"day\":null,\"date\":\"2026-11-18\",\"why\":\"fällig\",\"id\":\"o1\",\"title\":\"Tax\"},
    {\"section_kind\":\"day\",\"day\":\"2026-11-21\",\"date\":\"09:00\",\"why\":\"termin\",\"id\":\"a1\",\"title\":\"Dentist\"},
    {\"section_kind\":\"day\",\"day\":\"2026-11-21\",\"date\":\"2026-11-21\",\"why\":\"geplant\",\"id\":\"t1\",\"title\":\"Milk [2 l] now\"},
    {\"section_kind\":\"flagged\",\"day\":null,\"date\":\"\",\"why\":\"markiert\",\"id\":\"f1\",\"title\":\"Flag\"},
    {\"section_kind\":\"day\",\"day\":\"2026-11-23\",\"date\":\"2026-11-23\",\"why\":\"fällig\",\"id\":\"t2\",\"title\":\"Shelf\"},
    {\"section_kind\":\"later\",\"day\":null,\"date\":\"2026-12-01\",\"why\":\"fällig\",\"id\":\"t3\",\"title\":\"Report\"}]"
  "A forecast: overdue, an appointment, two days, flagged, later.")

(ert-deftest samu-test-agenda-entries-of-a-day ()
  (require 'ol)
  (let ((rows (samu--parse samu-test--forecast)))
    (should (equal (samu--agenda-entries "2026-11-21" "2026-11-21" rows)
                   '("Overdue 2026-11-18: [[samu:o1][Tax]]"
                     "Planned: [[samu:t1][Milk [2 l] now]]")))
    (should (equal (samu--agenda-entries "2026-11-23" "2026-11-21" rows)
                   '("Due: [[samu:t2][Shelf]]")))
    ;; beyond the days the forecast lists one by one
    (should (equal (samu--agenda-entries "2026-12-01" "2026-11-21" rows)
                   '("Due: [[samu:t3][Report]]")))
    (should (null (samu--agenda-entries "2026-11-22" "2026-11-21" rows)))))

(ert-deftest samu-test-agenda-asks-once-for-a-week ()
  (samu-test--with-cli `((("forecast") 0 ,samu-test--forecast ""))
    (let ((samu--agenda nil))
      (with-no-warnings (defvar date))
      (dolist (day '(21 22 23))
        (let ((date (list 11 day 2026)))
          (samu-org-agenda)))
      (let ((date '(11 23 2026)))
        (should (equal (samu-org-agenda) '("Due: [[samu:t2][Shelf]]"))))
      (should (= (length (samu-test--called '("forecast"))) 1))
      ;; a change here or from elsewhere asks again
      (cl-letf (((symbol-function 'samu--live-buffers) (lambda () nil)))
        (samu--refresh-all))
      (let ((date '(11 23 2026))) (samu-org-agenda))
      (should (= (length (samu-test--called '("forecast"))) 2)))))

(ert-deftest samu-test-agenda-without-network-is-empty-and-says-so-once ()
  (samu-test--with-cli '((("forecast") 1 "" "samu: no network"))
    (let ((samu--agenda nil))
      (with-no-warnings (defvar date))
      (dolist (day '(21 22))
        (let ((date (list 11 day 2026)))
          (should (null (samu-org-agenda)))))
      (should (= (length (samu-test--called '("forecast"))) 1)))))
