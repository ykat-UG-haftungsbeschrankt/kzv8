#include "os_platform.h"

#include <array>
#include <atomic>
#include <cstddef>
#include <cstdint>
#include <limits>
#include <memory>
#include <new>
#include <utility>

#if !defined(KZ_FREESTANDING)
#include <pthread.h>
#include <time.h>
#endif

#include "libplatform/libplatform.h"

extern "C" {
uint64_t kz_host_time_ns();
int64_t kz_host_wall_time_ms();
#if defined(KZ_FREESTANDING)
void kz_host_wait_for_interrupt();
void* kz_host_v8_random_mmap_addr();
void* kz_host_v8_allocate_pages(void* hint, size_t length, size_t alignment,
																int permissions);
int kz_host_v8_free_pages(void* address, size_t length);
int kz_host_v8_release_pages(void* address, size_t length, size_t new_length);
int kz_host_v8_decommit_pages(void* address, size_t length);
int kz_host_v8_set_page_permissions(void* address, size_t length,
																		int permissions);
#endif
}

#if defined(KZ_FREESTANDING)
struct RuntimeWorkerMutex {
	uintptr_t owner;
};

struct RuntimeWorkerCondition {
	uintptr_t generation;
};

using RuntimeWorkerThread = uintptr_t;

extern "C" {
int kz_worker_mutex_init(void*, const void*) __asm__("pthread_mutex_init");
int kz_worker_mutex_destroy(void*) __asm__("pthread_mutex_destroy");
int kz_worker_mutex_lock(void*) __asm__("pthread_mutex_lock");
int kz_worker_mutex_unlock(void*) __asm__("pthread_mutex_unlock");
int kz_worker_condition_init(void*, const void*) __asm__("pthread_cond_init");
int kz_worker_condition_destroy(void*) __asm__("pthread_cond_destroy");
int kz_worker_condition_signal(void*) __asm__("pthread_cond_signal");
int kz_worker_condition_broadcast(void*) __asm__("pthread_cond_broadcast");
int kz_worker_thread_create(uintptr_t*, const void*, void* (*)(void*), void*)
		__asm__("pthread_create");
int kz_worker_thread_join(uintptr_t, void**) __asm__("pthread_join");
}

inline int runtime_worker_mutex_init(RuntimeWorkerMutex* mutex) {
	return kz_worker_mutex_init(mutex, nullptr);
}
inline int runtime_worker_mutex_destroy(RuntimeWorkerMutex* mutex) {
	return kz_worker_mutex_destroy(mutex);
}
inline int runtime_worker_mutex_lock(RuntimeWorkerMutex* mutex) {
	return kz_worker_mutex_lock(mutex);
}
inline int runtime_worker_mutex_unlock(RuntimeWorkerMutex* mutex) {
	return kz_worker_mutex_unlock(mutex);
}
inline int runtime_worker_condition_init(RuntimeWorkerCondition* condition) {
	return kz_worker_condition_init(condition, nullptr);
}
inline int runtime_worker_condition_destroy(RuntimeWorkerCondition* condition) {
	return kz_worker_condition_destroy(condition);
}
inline int runtime_worker_condition_signal(RuntimeWorkerCondition* condition) {
	return kz_worker_condition_signal(condition);
}
inline int runtime_worker_condition_broadcast(RuntimeWorkerCondition* condition) {
	return kz_worker_condition_broadcast(condition);
}
inline int runtime_worker_thread_create(RuntimeWorkerThread* thread,
													 void* (*entry)(void*), void* context) {
	return kz_worker_thread_create(thread, nullptr, entry, context);
}
inline int runtime_worker_thread_join(RuntimeWorkerThread thread) {
	return kz_worker_thread_join(thread, nullptr);
}
#else
using RuntimeWorkerMutex = pthread_mutex_t;
using RuntimeWorkerCondition = pthread_cond_t;
using RuntimeWorkerThread = pthread_t;

inline int runtime_worker_mutex_init(RuntimeWorkerMutex* mutex) {
	return pthread_mutex_init(mutex, nullptr);
}
inline int runtime_worker_mutex_destroy(RuntimeWorkerMutex* mutex) {
	return pthread_mutex_destroy(mutex);
}
inline int runtime_worker_mutex_lock(RuntimeWorkerMutex* mutex) {
	return pthread_mutex_lock(mutex);
}
inline int runtime_worker_mutex_unlock(RuntimeWorkerMutex* mutex) {
	return pthread_mutex_unlock(mutex);
}
inline int runtime_worker_condition_init(RuntimeWorkerCondition* condition) {
	return pthread_cond_init(condition, nullptr);
}
inline int runtime_worker_condition_destroy(RuntimeWorkerCondition* condition) {
	return pthread_cond_destroy(condition);
}
inline int runtime_worker_condition_signal(RuntimeWorkerCondition* condition) {
	return pthread_cond_signal(condition);
}
inline int runtime_worker_condition_broadcast(RuntimeWorkerCondition* condition) {
	return pthread_cond_broadcast(condition);
}
inline int runtime_worker_thread_create(RuntimeWorkerThread* thread,
													 void* (*entry)(void*), void* context) {
	return pthread_create(thread, nullptr, entry, context);
}
inline int runtime_worker_thread_join(RuntimeWorkerThread thread) {
	return pthread_join(thread, nullptr);
}
#endif

namespace {

constexpr size_t kTaskCapacity = 128;
constexpr uint64_t kNoDeadline = std::numeric_limits<uint64_t>::max();
constexpr double kNanosecondsPerSecond = 1'000'000'000.0;

[[noreturn]] void task_queue_overflow() {
	__builtin_trap();
}

class MutexLock {
 public:
	explicit MutexLock(RuntimeWorkerMutex& mutex) : mutex_(mutex) {
		if (runtime_worker_mutex_lock(&mutex_) != 0) {
			task_queue_overflow();
		}
	}

	~MutexLock() {
		if (runtime_worker_mutex_unlock(&mutex_) != 0) {
			task_queue_overflow();
		}
	}

	MutexLock(const MutexLock&) = delete;
	MutexLock& operator=(const MutexLock&) = delete;

 private:
	RuntimeWorkerMutex& mutex_;
};

uint64_t deadline_from_delay(uint64_t now_ns, double delay_seconds) {
	if (!(delay_seconds > 0.0)) {
		return now_ns;
	}
	if (delay_seconds >= 18'446'744'073.0) return kNoDeadline;

	const double delay_ns = delay_seconds * kNanosecondsPerSecond;
	const uint64_t remaining = kNoDeadline - now_ns;
	if (delay_ns >= static_cast<double>(remaining)) {
		return kNoDeadline;
	}
	return now_ns + static_cast<uint64_t>(delay_ns);
}

#if defined(KZ_FREESTANDING)
class KernelPageAllocator final : public v8::PageAllocator {
 public:
	size_t AllocatePageSize() override { return 4096; }
	size_t CommitPageSize() override { return 4096; }
	void SetRandomMmapSeed(int64_t) override {}
	void* GetRandomMmapAddr() override { return kz_host_v8_random_mmap_addr(); }

	void* AllocatePages(void* hint, size_t length, size_t alignment,
											Permission permissions) override {
		return kz_host_v8_allocate_pages(hint, length, alignment,
																		 static_cast<int>(permissions));
	}

	bool FreePages(void* address, size_t length) override {
		return kz_host_v8_free_pages(address, length) != 0;
	}

	bool ReleasePages(void* address, size_t length,
										size_t new_length) override {
		return kz_host_v8_release_pages(address, length, new_length) != 0;
	}

	bool SetPermissions(void* address, size_t length,
											Permission permissions) override {
		return kz_host_v8_set_page_permissions(
							 address, length, static_cast<int>(permissions)) != 0;
	}

	bool DecommitPages(void* address, size_t length) override {
		return kz_host_v8_decommit_pages(address, length) != 0;
	}
};
#endif

class CooperativeTaskRunner final : public v8::TaskRunner {
 public:
	CooperativeTaskRunner() {
		if (runtime_worker_mutex_init(&mutex_) != 0) {
			task_queue_overflow();
		}
	}

	~CooperativeTaskRunner() override {
		if (runtime_worker_mutex_destroy(&mutex_) != 0) {
			task_queue_overflow();
		}
	}

	bool Enqueue(std::unique_ptr<v8::Task> task, uint64_t deadline_ns) {
		MutexLock lock(mutex_);
		for (PendingTask& pending : tasks_) {
			if (pending.task) {
				continue;
			}
			pending.task = std::move(task);
			pending.deadline_ns = deadline_ns;
			pending.sequence = next_sequence_++;
			return true;
		}
		task_queue_overflow();
	}

	bool PumpOne() {
		std::unique_ptr<v8::Task> task;
		{
			MutexLock lock(mutex_);
			const uint64_t now_ns = kz_host_time_ns();
			size_t selected = tasks_.size();
			for (size_t index = 0; index < tasks_.size(); index++) {
				const PendingTask& candidate = tasks_[index];
				if (!candidate.task || candidate.deadline_ns > now_ns) {
					continue;
				}
				if (selected == tasks_.size() ||
						candidate.deadline_ns < tasks_[selected].deadline_ns ||
						(candidate.deadline_ns == tasks_[selected].deadline_ns &&
						 candidate.sequence < tasks_[selected].sequence)) {
					selected = index;
				}
			}

			if (selected == tasks_.size()) {
				return false;
			}

			task = std::move(tasks_[selected].task);
			tasks_[selected].deadline_ns = kNoDeadline;
			tasks_[selected].sequence = 0;
		}

		task->Run();
		return true;
	}

	uint64_t NextDeadlineNs() const {
		MutexLock lock(mutex_);
		uint64_t deadline_ns = kNoDeadline;
		for (const PendingTask& task : tasks_) {
			if (task.task && task.deadline_ns < deadline_ns) {
				deadline_ns = task.deadline_ns;
			}
		}
		return deadline_ns;
	}

	bool IdleTasksEnabled() override { return false; }
	bool NonNestableTasksEnabled() const override { return true; }
	bool NonNestableDelayedTasksEnabled() const override { return true; }

 protected:
	void PostTaskImpl(std::unique_ptr<v8::Task> task,
										const v8::SourceLocation&) override {
		Enqueue(std::move(task), kz_host_time_ns());
	}

	void PostNonNestableTaskImpl(std::unique_ptr<v8::Task> task,
															 const v8::SourceLocation&) override {
		Enqueue(std::move(task), kz_host_time_ns());
	}

	void PostDelayedTaskImpl(std::unique_ptr<v8::Task> task,
													 double delay_in_seconds,
													 const v8::SourceLocation&) override {
		Enqueue(std::move(task),
						deadline_from_delay(kz_host_time_ns(), delay_in_seconds));
	}

	void PostNonNestableDelayedTaskImpl(
			std::unique_ptr<v8::Task> task, double delay_in_seconds,
			const v8::SourceLocation&) override {
		Enqueue(std::move(task),
						deadline_from_delay(kz_host_time_ns(), delay_in_seconds));
	}

 private:
	struct PendingTask {
		std::unique_ptr<v8::Task> task;
		uint64_t deadline_ns = kNoDeadline;
		uint64_t sequence = 0;
	};

	mutable RuntimeWorkerMutex mutex_{};
	std::array<PendingTask, kTaskCapacity> tasks_{};
	uint64_t next_sequence_ = 0;
};

class WorkerPool {
 public:
	~WorkerPool() { Shutdown(); }

	bool Start() {
		if (runtime_worker_mutex_init(&mutex_) != 0) {
			return false;
		}
		mutex_initialized_ = true;
		if (runtime_worker_condition_init(&condition_) != 0) {
			DestroySynchronization();
			return false;
		}
		condition_initialized_ = true;
		if (runtime_worker_thread_create(&worker_, WorkerEntry, this) != 0) {
			DestroySynchronization();
			return false;
		}
		worker_started_ = true;
		worker_count_.store(1, std::memory_order_release);
		return true;
	}

	size_t WorkerCount() const {
		return worker_count_.load(std::memory_order_acquire);
	}

	bool Enqueue(std::unique_ptr<v8::Task>&& task, uint64_t deadline_ns) {
		if (WorkerCount() == 0) {
			return false;
		}
		MutexLock lock(mutex_);
		if (stopping_) {
			return false;
		}
		for (PendingTask& pending : tasks_) {
			if (pending.task) {
				continue;
			}
			pending.task = std::move(task);
			pending.deadline_ns = deadline_ns;
			pending.sequence = next_sequence_++;
			if (runtime_worker_condition_signal(&condition_) != 0) {
				task_queue_overflow();
			}
			return true;
		}
		task_queue_overflow();
	}

	void Shutdown() {
		if (!mutex_initialized_) {
			return;
		}
		if (worker_started_) {
			if (runtime_worker_mutex_lock(&mutex_) != 0) {
				task_queue_overflow();
			}
			stopping_ = true;
			if (runtime_worker_condition_broadcast(&condition_) != 0 ||
					runtime_worker_mutex_unlock(&mutex_) != 0) {
				task_queue_overflow();
			}
			if (runtime_worker_thread_join(worker_) != 0) {
				task_queue_overflow();
			}
			worker_started_ = false;
			worker_count_.store(0, std::memory_order_release);
			for (PendingTask& pending : tasks_) {
				pending.task.reset();
			}
		}
		DestroySynchronization();
	}

 private:
	struct PendingTask {
		std::unique_ptr<v8::Task> task;
		uint64_t deadline_ns = kNoDeadline;
		uint64_t sequence = 0;
	};

	static void* WorkerEntry(void* context) {
		static_cast<WorkerPool*>(context)->WorkerLoop();
		return nullptr;
	}

	void WorkerLoop() {
		for (;;) {
			if (runtime_worker_mutex_lock(&mutex_) != 0) {
				task_queue_overflow();
			}
			if (stopping_) {
				if (runtime_worker_mutex_unlock(&mutex_) != 0) {
					task_queue_overflow();
				}
				return;
			}

			const uint64_t now_ns = kz_host_time_ns();
			size_t selected = tasks_.size();
			for (size_t index = 0; index < tasks_.size(); index++) {
				const PendingTask& candidate = tasks_[index];
				if (!candidate.task || candidate.deadline_ns > now_ns) {
					continue;
				}
				if (selected == tasks_.size() ||
						candidate.deadline_ns < tasks_[selected].deadline_ns ||
						(candidate.deadline_ns == tasks_[selected].deadline_ns &&
						 candidate.sequence < tasks_[selected].sequence)) {
					selected = index;
				}
			}

			if (selected == tasks_.size()) {
				const uint64_t deadline_ns = NextDeadlineNsLocked();
#if defined(KZ_FREESTANDING)
				if (runtime_worker_mutex_unlock(&mutex_) != 0) {
					task_queue_overflow();
				}
				if (deadline_ns == kNoDeadline || kz_host_time_ns() < deadline_ns) {
					kz_host_wait_for_interrupt();
				}
#else
				if (deadline_ns == kNoDeadline) {
					static_cast<void>(pthread_cond_wait(&condition_, &mutex_));
				} else {
					timespec wake_time{};
					if (clock_gettime(CLOCK_REALTIME, &wake_time) == 0) {
						const uint64_t remaining_ns =
								deadline_ns > now_ns ? deadline_ns - now_ns : 0;
						const uint64_t absolute_ns =
								static_cast<uint64_t>(wake_time.tv_nsec) + remaining_ns;
						wake_time.tv_sec +=
								static_cast<time_t>(absolute_ns / 1'000'000'000);
						wake_time.tv_nsec =
								static_cast<long>(absolute_ns % 1'000'000'000);
						static_cast<void>(pthread_cond_timedwait(
								&condition_, &mutex_, &wake_time));
					} else {
						static_cast<void>(pthread_cond_wait(&condition_, &mutex_));
					}
				}
				if (runtime_worker_mutex_unlock(&mutex_) != 0) {
					task_queue_overflow();
				}
#endif
				continue;
			}

			std::unique_ptr<v8::Task> task = std::move(tasks_[selected].task);
			tasks_[selected].deadline_ns = kNoDeadline;
			tasks_[selected].sequence = 0;
			if (runtime_worker_mutex_unlock(&mutex_) != 0) {
				task_queue_overflow();
			}
			task->Run();
		}
	}

	uint64_t NextDeadlineNsLocked() const {
		uint64_t deadline_ns = kNoDeadline;
		for (const PendingTask& task : tasks_) {
			if (task.task && task.deadline_ns < deadline_ns) {
				deadline_ns = task.deadline_ns;
			}
		}
		return deadline_ns;
	}

	void DestroySynchronization() {
		if (condition_initialized_) {
			if (runtime_worker_condition_destroy(&condition_) != 0) {
				task_queue_overflow();
			}
			condition_initialized_ = false;
		}
		if (mutex_initialized_) {
			if (runtime_worker_mutex_destroy(&mutex_) != 0) {
				task_queue_overflow();
			}
			mutex_initialized_ = false;
		}
	}

	RuntimeWorkerMutex mutex_{};
	RuntimeWorkerCondition condition_{};
	RuntimeWorkerThread worker_{};
	std::array<PendingTask, kTaskCapacity> tasks_{};
	std::atomic<size_t> worker_count_{0};
	uint64_t next_sequence_ = 0;
	bool mutex_initialized_ = false;
	bool condition_initialized_ = false;
	bool worker_started_ = false;
	bool stopping_ = false;
};

class NoOpTracingController final : public v8::TracingController {};

class OsPlatform final : public v8::Platform {
 public:
	OsPlatform(v8::PageAllocator* page_allocator, bool use_kernel_allocator)
			: use_kernel_allocator_(use_kernel_allocator),
				external_page_allocator_(page_allocator),
				task_runner_(std::make_shared<CooperativeTaskRunner>()) {
		static_cast<void>(worker_pool_.Start());
	}

	~OsPlatform() override { ShutdownWorkers(); }

	v8::PageAllocator* GetPageAllocator() override {
		if (external_page_allocator_ != nullptr) {
			return external_page_allocator_;
		}
#if defined(KZ_FREESTANDING)
		return use_kernel_allocator_ ? &kernel_page_allocator_ : nullptr;
#else
	static_cast<void>(use_kernel_allocator_);
		return nullptr;
#endif
	}
	int NumberOfWorkerThreads() override {
		return static_cast<int>(worker_pool_.WorkerCount());
	}

	std::shared_ptr<v8::TaskRunner> GetForegroundTaskRunner(
			v8::Isolate*, v8::TaskPriority) override {
		return task_runner_;
	}

	double MonotonicallyIncreasingTime() override {
		return static_cast<double>(kz_host_time_ns()) / kNanosecondsPerSecond;
	}

	double CurrentClockTimeMillis() override {
		return static_cast<double>(kz_host_wall_time_ms());
	}

	v8::TracingController* GetTracingController() override { return &tracing_; }

	std::unique_ptr<v8::JobHandle> CreateJobImpl(
			v8::TaskPriority priority, std::unique_ptr<v8::JobTask> task,
			const v8::SourceLocation&) override {
		return v8::platform::NewDefaultJobHandle(
				this, priority, std::move(task), worker_pool_.WorkerCount());
	}

	bool PumpOne() { return task_runner_->PumpOne(); }
	uint64_t NextDeadlineNs() const { return task_runner_->NextDeadlineNs(); }
	void ShutdownWorkers() { worker_pool_.Shutdown(); }

 protected:
	void PostTaskOnWorkerThreadImpl(v8::TaskPriority,
																	std::unique_ptr<v8::Task> task,
																	const v8::SourceLocation&) override {
		if (!worker_pool_.Enqueue(std::move(task), kz_host_time_ns())) {
			task_runner_->Enqueue(std::move(task), kz_host_time_ns());
		}
	}

	void PostDelayedTaskOnWorkerThreadImpl(
			v8::TaskPriority, std::unique_ptr<v8::Task> task,
			double delay_in_seconds, const v8::SourceLocation&) override {
		const uint64_t deadline_ns =
				deadline_from_delay(kz_host_time_ns(), delay_in_seconds);
		if (!worker_pool_.Enqueue(std::move(task), deadline_ns)) {
			task_runner_->Enqueue(std::move(task), deadline_ns);
		}
	}

 private:
	bool use_kernel_allocator_;
#if defined(KZ_FREESTANDING)
	KernelPageAllocator kernel_page_allocator_;
#endif
	v8::PageAllocator* external_page_allocator_;
	std::shared_ptr<CooperativeTaskRunner> task_runner_;
	WorkerPool worker_pool_;
	NoOpTracingController tracing_;
};

}  // namespace

extern "C" v8::Platform* kz_os_platform_create(
		v8::PageAllocator* page_allocator) {
	return new (std::nothrow) OsPlatform(page_allocator, false);
}

extern "C" v8::Platform* kz_os_platform_create_for_kernel() {
	return new (std::nothrow) OsPlatform(nullptr, true);
}

extern "C" void kz_os_platform_destroy(v8::Platform* platform) {
	delete static_cast<OsPlatform*>(platform);
}

extern "C" void kz_os_platform_shutdown_workers(v8::Platform* platform) {
	if (platform != nullptr) {
		static_cast<OsPlatform*>(platform)->ShutdownWorkers();
	}
}

extern "C" int kz_os_platform_pump_one(v8::Platform* platform) {
	return platform != nullptr && static_cast<OsPlatform*>(platform)->PumpOne();
}

extern "C" uint64_t kz_os_platform_next_deadline_ns(v8::Platform* platform) {
	return platform == nullptr ? kNoDeadline
															: static_cast<OsPlatform*>(platform)->NextDeadlineNs();
}
