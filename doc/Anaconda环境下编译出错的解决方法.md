官方说明有误，故按照其流程无法在 Anaconda 下成功编译 ray 。详见[ray-project/ray #6256](https://github.com/ray-project/ray/issues/6256)。

## 原因

Anaconda 中的 gcc 使用 `--enable-gold=yes` 编译，使得低版本 bazel(<2.0.0) 无法探测到正确的 gold linker 支持选项，从而附加错误的编译参数。由于 ray 还在使用 1.x 版本的 bazel ，所以需要手动 patch 出问题的文件。

## 解决方法

下载 bazel 1.2.1 的源码，加入如下两个补丁：

1. grpc gettid patch

```diff
From d1d017390b799c59d6fdf7b8afa6136d218bdd61 Mon Sep 17 00:00:00 2001
From: Benjamin Peterson <benjamin@dropbox.com>
Date: Fri, 3 May 2019 08:11:00 -0700
Subject: [PATCH] Rename gettid() functions.

glibc 2.30 will declare its own gettid; see https://sourceware.org/git/?p=glibc.git;a=commit;h=1d0fc213824eaa2a8f8c4385daaa698ee8fb7c92. Rename the grpc versions to avoid naming conflicts.
---
 src/core/lib/gpr/log_linux.cc          | 4 ++--
 src/core/lib/gpr/log_posix.cc          | 4 ++--
 src/core/lib/iomgr/ev_epollex_linux.cc | 4 ++--
 3 files changed, 6 insertions(+), 6 deletions(-)

diff --git src/core/lib/gpr/log_linux.cc.orig src/core/lib/gpr/log_linux.cc
index 561276f0c20..8b597b4cf2f 100644
--- src/core/lib/gpr/log_linux.cc.orig	2019-06-21 10:42:13.235611417 -0400
+++ src/core/lib/gpr/log_linux.cc	2019-06-21 10:42:16.686608855 -0400
@@ -40,7 +40,7 @@
 #include <time.h>
 #include <unistd.h>
 
-static long gettid(void) { return syscall(__NR_gettid); }
+static long sys_gettid(void) { return syscall(__NR_gettid); }
 
 void gpr_log(const char* file, int line, gpr_log_severity severity,
              const char* format, ...) {
@@ -70,7 +70,7 @@ void gpr_default_log(gpr_log_func_args*
   gpr_timespec now = gpr_now(GPR_CLOCK_REALTIME);
   struct tm tm;
   static __thread long tid = 0;
-  if (tid == 0) tid = gettid();
+  if (tid == 0) tid = sys_gettid();
 
   timer = static_cast<time_t>(now.tv_sec);
   final_slash = strrchr(args->file, '/');
diff --git src/core/lib/gpr/log_posix.cc.orig src/core/lib/gpr/log_posix.cc
index b6edc14ab6b..2f7c6ce3760 100644
--- src/core/lib/gpr/log_posix.cc.orig	2019-06-21 10:42:13.242611412 -0400
+++ src/core/lib/gpr/log_posix.cc	2019-06-21 10:42:22.794604319 -0400
@@ -30,7 +30,7 @@
 #include <string.h>
 #include <time.h>
 
-static intptr_t gettid(void) { return (intptr_t)pthread_self(); }
+static intptr_t sys_gettid(void) { return (intptr_t)pthread_self(); }
 
 void gpr_log(const char* file, int line, gpr_log_severity severity,
              const char* format, ...) {
@@ -85,7 +85,7 @@ void gpr_default_log(gpr_log_func_args*
   char* prefix;
   gpr_asprintf(&prefix, "%s%s.%09d %7tu %s:%d]",
                gpr_log_severity_string(args->severity), time_buffer,
-               (int)(now.tv_nsec), gettid(), display_file, args->line);
+               (int)(now.tv_nsec), sys_gettid(), display_file, args->line);
 
   fprintf(stderr, "%-70s %s\n", prefix, args->message);
   gpr_free(prefix);
diff --git src/core/lib/iomgr/ev_epollex_linux.cc.orig src/core/lib/iomgr/ev_epollex_linux.cc
index 08116b3ab53..76f59844312 100644
--- src/core/lib/iomgr/ev_epollex_linux.cc.orig	2019-06-21 10:42:13.247611408 -0400
+++ src/core/lib/iomgr/ev_epollex_linux.cc	2019-06-21 10:42:29.767599141 -0400
@@ -1150,7 +1150,7 @@ static void end_worker(grpc_pollset* pol
 }
 
 #ifndef NDEBUG
-static long gettid(void) { return syscall(__NR_gettid); }
+static long sys_gettid(void) { return syscall(__NR_gettid); }
 #endif
 
 /* pollset->mu lock must be held by the caller before calling this.
@@ -1170,7 +1170,7 @@ static grpc_error* pollset_work(grpc_pol
 #define WORKER_PTR (&worker)
 #endif
 #ifndef NDEBUG
-  WORKER_PTR->originator = gettid();
+  WORKER_PTR->originator = sys_gettid();
 #endif
   if (grpc_polling_trace.enabled()) {
     gpr_log(GPR_INFO,

```

2. gold linker patch

```diff
From f31ee4d7904c0d5d0b42a53b85f4053f205a3657 Mon Sep 17 00:00:00 2001
From: hlopko <hlopko@google.com>
Date: Fri, 29 Nov 2019 01:51:50 -0800
Subject: [PATCH] Skip gcc build configuration options mentioning gold in
 ld.gold detection

Fixes bazelbuild/bazel#9392

rules_cc version of this change is in https://github.com/bazelbuild/rules_cc/pull/48.

RELNOTES: None.
PiperOrigin-RevId: 283025854
---
 tools/cpp/unix_cc_configure.bzl | 4 ++++
 1 file changed, 4 insertions(+)

diff --git tools/cpp/unix_cc_configure.bzl.orig tools/cpp/unix_cc_configure.bzl
index ccb18a431a3..0c72f770f9e 100644
--- tools/cpp/unix_cc_configure.bzl.orig
+++ tools/cpp/unix_cc_configure.bzl
@@ -200,6 +200,10 @@ def _find_gold_linker_path(repository_ctx, cc):
         for flag in line.split(" "):
             if flag.find("gold") == -1:
                 continue
+            if flag.find("--enable-gold") > -1 or flag.find("--with-plugin-ld") > -1:
+                # skip build configuration options of gcc itself
+                # TODO(hlopko): Add redhat-like worker on the CI (#9392)
+                continue
 
             # flag is '-fuse-ld=gold' for GCC or "/usr/lib/ld.gold" for Clang
             # strip space, single quote, and double quotes
```

之后编译安装 bazel ，并在设置 Anaconda 的环境下运行 build.sh 。