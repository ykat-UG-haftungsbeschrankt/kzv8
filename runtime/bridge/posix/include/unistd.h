/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

#ifndef KZ_POSIX_UNISTD_H_
#define KZ_POSIX_UNISTD_H_

#include <fcntl.h>
#include <stddef.h>
#include <stdint.h>
#include <sys/types.h>

#define F_OK 0
#define R_OK 4
#define W_OK 2
#define X_OK 1

#define STDIN_FILENO 0
#define STDOUT_FILENO 1
#define STDERR_FILENO 2

#define SEEK_DATA 3
#define SEEK_HOLE 4

#define _POSIX_VDISABLE 0
#define POSIX_CLOSE_RESTART 1

#define _CS_PATH 0

#define _SC_CLK_TCK 2
#define _SC_PAGESIZE 30
#define _SC_PAGE_SIZE _SC_PAGESIZE
#define _SC_NPROCESSORS_CONF 83
#define _SC_NPROCESSORS_ONLN 84

#ifdef __cplusplus
extern "C" {
#endif

extern char **environ;
extern char *optarg;
extern int opterr;
extern int optind;
extern int optopt;

int access(const char *path, int mode);
unsigned int alarm(unsigned int seconds);
int chdir(const char *path);
int chown(const char *path, uid_t owner, gid_t group);
size_t confstr(int name, char *buffer, size_t size);
char *crypt(const char *key, const char *salt);
char *ctermid(char *buffer);
int dup(int file_descriptor);
int dup2(int file_descriptor, int new_descriptor);
int dup3(int file_descriptor, int new_descriptor, int flags);
void _exit(int status) __attribute__((noreturn));
int execl(const char *path, const char *argument, ...);
int execle(const char *path, const char *argument, ...);
int execlp(const char *file, const char *argument, ...);
int execv(const char *path, char *const arguments[]);
int execve(const char *path, char *const arguments[], char *const environment[]);
int execvp(const char *file, char *const arguments[]);
int faccessat(int directory_descriptor, const char *path, int mode, int flags);
int fchdir(int file_descriptor);
int fchown(int file_descriptor, uid_t owner, gid_t group);
int fchownat(int directory_descriptor, const char *path, uid_t owner, gid_t group, int flags);
int fdatasync(int file_descriptor);
int fexecve(int file_descriptor, char *const arguments[], char *const environment[]);
pid_t _Fork(void);
pid_t fork(void);
long fpathconf(int file_descriptor, int name);
int fsync(int file_descriptor);
int ftruncate(int file_descriptor, off_t length);
char *getcwd(char *buffer, size_t size);
gid_t getegid(void);
uid_t geteuid(void);
gid_t getgid(void);
int getgroups(int group_count, gid_t groups[]);
int getentropy(void *buffer, size_t size);
long gethostid(void);
int gethostname(char *name, size_t size);
char *getlogin(void);
int getlogin_r(char *buffer, size_t size);
int getopt(int argument_count, char *const arguments[], const char *options);
pid_t getpgid(pid_t process);
pid_t getpgrp(void);
pid_t getpid(void);
pid_t getppid(void);
int getresgid(gid_t *real_group, gid_t *effective_group, gid_t *saved_group);
int getresuid(uid_t *real_user, uid_t *effective_user, uid_t *saved_user);
pid_t getsid(pid_t process);
uid_t getuid(void);
int isatty(int file_descriptor);
int lchown(const char *path, uid_t owner, gid_t group);
int link(const char *existing_path, const char *new_path);
int linkat(int old_directory_descriptor, const char *old_path,
           int new_directory_descriptor, const char *new_path, int flags);
int lockf(int file_descriptor, int command, off_t length);
off_t lseek(int file_descriptor, off_t offset, int origin);
int nice(int increment);
long pathconf(const char *path, int name);
int pause(void);
int pipe(int file_descriptors[2]);
int pipe2(int file_descriptors[2], int flags);
int posix_close(int file_descriptor, int flags);
ssize_t pread(int file_descriptor, void *buffer, size_t size, off_t offset);
ssize_t pwrite(int file_descriptor, const void *buffer, size_t size, off_t offset);
ssize_t read(int file_descriptor, void *buffer, size_t size);
ssize_t readlink(const char *path, char *buffer, size_t size);
ssize_t readlinkat(int directory_descriptor, const char *path,
                   char *buffer, size_t size);
int rmdir(const char *path);
int setegid(gid_t group);
int seteuid(uid_t user);
int setgid(gid_t group);
int setpgid(pid_t process, pid_t group);
int setpgrp(void);
int setregid(gid_t real_group, gid_t effective_group);
int setresgid(gid_t real_group, gid_t effective_group, gid_t saved_group);
int setresuid(uid_t real_user, uid_t effective_user, uid_t saved_user);
int setreuid(uid_t real_user, uid_t effective_user);
pid_t setsid(void);
int setuid(uid_t user);
unsigned int sleep(unsigned int seconds);
void swab(const void *source, void *destination, ssize_t size);
int symlink(const char *target, const char *link_path);
int symlinkat(const char *target, int directory_descriptor, const char *link_path);
void sync(void);
long sysconf(int name);
pid_t tcgetpgrp(int file_descriptor);
int tcsetpgrp(int file_descriptor, pid_t process_group);
int truncate(const char *path, off_t length);
char *ttyname(int file_descriptor);
int ttyname_r(int file_descriptor, char *buffer, size_t size);
int unlink(const char *path);
int unlinkat(int directory_descriptor, const char *path, int flags);
ssize_t write(int file_descriptor, const void *buffer, size_t size);

void encrypt(char block[64], int flag);

#ifdef __cplusplus
}
#endif

#endif