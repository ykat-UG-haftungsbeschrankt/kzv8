#include <pthread.h>

static pthread_mutex_t smoke_mutex = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t smoke_condition = PTHREAD_COND_INITIALIZER;
static int smoke_ready;
static volatile int smoke_finished;

static void *smoke_worker(void *argument) {
    if (pthread_mutex_lock(&smoke_mutex) != 0) {
        return 0;
    }
    smoke_ready = 1;
    int signal_status = pthread_cond_signal(&smoke_condition);
    int unlock_status = pthread_mutex_unlock(&smoke_mutex);
    return signal_status == 0 && unlock_status == 0 ? argument : 0;
}

static void *smoke_completion_worker(void *argument) {
    smoke_finished += 1;
    return argument;
}

int kz_c_pthread_smoke(void) {
    pthread_attr_t attributes;
    if (pthread_attr_init(&attributes) != 0
        || pthread_attr_setdetachstate(&attributes, PTHREAD_CREATE_JOINABLE) != 0) {
        return -1;
    }

    pthread_t thread;
    if (pthread_create(&thread, &attributes, smoke_worker, (void *)(uintptr_t)42) != 0) {
        return -1;
    }

    if (pthread_mutex_lock(&smoke_mutex) != 0) {
        return -1;
    }
    while (!smoke_ready) {
        if (pthread_cond_wait(&smoke_condition, &smoke_mutex) != 0) {
            return -1;
        }
    }
    if (pthread_mutex_unlock(&smoke_mutex) != 0) {
        return -1;
    }

    void *result = 0;
    if (pthread_join(thread, &result) != 0 || result != (void *)(uintptr_t)42) {
        return -1;
    }
    if (pthread_mutex_destroy(&smoke_mutex) != 0
        || pthread_cond_destroy(&smoke_condition) != 0) {
        return -1;
    }

    smoke_finished = 0;
    if (pthread_attr_setdetachstate(&attributes, PTHREAD_CREATE_DETACHED) != 0
        || pthread_create(&thread, &attributes, smoke_completion_worker,
                          (void *)(uintptr_t)84) != 0) {
        return -1;
    }
    while (smoke_finished < 1) {
        sched_yield();
    }

    if (pthread_attr_setdetachstate(&attributes, PTHREAD_CREATE_JOINABLE) != 0
        || pthread_create(&thread, &attributes, smoke_completion_worker,
                          (void *)(uintptr_t)126) != 0
        || pthread_detach(thread) != 0) {
        return -1;
    }
    while (smoke_finished < 2) {
        sched_yield();
    }

    if (pthread_create(&thread, &attributes, smoke_completion_worker,
                       (void *)(uintptr_t)168) != 0
        || pthread_join(thread, &result) != 0
        || result != (void *)(uintptr_t)168) {
        return -1;
    }
    return pthread_attr_destroy(&attributes);
}