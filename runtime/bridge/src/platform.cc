#include "os_platform.h"

#include <array>
#include <cstddef>
#include <cstdint>
#include <limits>
#include <memory>
#include <new>
#include <utility>

extern "C" {
uint64_t kz_host_time_ns();
int64_t kz_host_wall_time_ms();
#if defined(KZ_FREESTANDING)
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

namespace {

constexpr size_t kTaskCapacity = 128;
constexpr uint64_t kNoDeadline = std::numeric_limits<uint64_t>::max();
constexpr double kNanosecondsPerSecond = 1'000'000'000.0;

[[noreturn]] void task_queue_overflow() {
	__builtin_trap();
}

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
	bool Enqueue(std::unique_ptr<v8::Task> task, uint64_t deadline_ns) {
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

		std::unique_ptr<v8::Task> task = std::move(tasks_[selected].task);
		tasks_[selected].deadline_ns = kNoDeadline;
		tasks_[selected].sequence = 0;
		task->Run();
		return true;
	}

	uint64_t NextDeadlineNs() const {
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

	std::array<PendingTask, kTaskCapacity> tasks_{};
	uint64_t next_sequence_ = 0;
};

struct JobState {
	explicit JobState(std::unique_ptr<v8::JobTask> job_task)
			: task(std::move(job_task)) {}

	std::unique_ptr<v8::JobTask> task;
	bool valid = true;
	bool cancelled = false;
	bool running = false;
	bool scheduled = false;
	bool schedule_again = false;
	bool joining = false;
};

bool job_is_active(const JobState& state) {
	return state.valid && !state.cancelled && state.task &&
				 state.task->GetMaxConcurrency(state.running ? 1 : 0) != 0;
}

class JobInvocation;
void schedule_job(const std::shared_ptr<JobState>& state,
									CooperativeTaskRunner* runner);
void run_job(const std::shared_ptr<JobState>& state,
						 CooperativeTaskRunner* runner);

class CooperativeJobDelegate final : public v8::JobDelegate {
 public:
	CooperativeJobDelegate(std::shared_ptr<JobState> state,
												 CooperativeTaskRunner* runner)
			: state_(std::move(state)), runner_(runner) {}

	bool ShouldYield() override {
		return state_->cancelled || !state_->task ||
					 state_->task->GetMaxConcurrency(1) == 0;
	}

	void NotifyConcurrencyIncrease() override {
		schedule_job(state_, runner_);
	}

	uint8_t GetTaskId() override { return 0; }
	bool IsJoiningThread() const override { return state_->joining; }

 private:
	std::shared_ptr<JobState> state_;
	CooperativeTaskRunner* runner_;
};

class JobInvocation final : public v8::Task {
 public:
	JobInvocation(std::shared_ptr<JobState> state,
								CooperativeTaskRunner* runner)
			: state_(std::move(state)), runner_(runner) {}

	void Run() override { run_job(state_, runner_); }

 private:
	std::shared_ptr<JobState> state_;
	CooperativeTaskRunner* runner_;
};

void schedule_job(const std::shared_ptr<JobState>& state,
									CooperativeTaskRunner* runner) {
	if (!job_is_active(*state)) {
		return;
	}
	if (state->running) {
		state->schedule_again = true;
		return;
	}
	if (state->scheduled) {
		return;
	}

	state->scheduled = true;
	runner->Enqueue(std::make_unique<JobInvocation>(state, runner),
									kz_host_time_ns());
}

void run_job(const std::shared_ptr<JobState>& state,
						 CooperativeTaskRunner* runner) {
	state->scheduled = false;
	if (!job_is_active(*state)) {
		return;
	}
	if (state->running) {
		state->schedule_again = true;
		return;
	}

	state->running = true;
	state->schedule_again = false;
	CooperativeJobDelegate delegate(state, runner);
	state->task->Run(&delegate);
	state->running = false;

	if (state->schedule_again || job_is_active(*state)) {
		state->schedule_again = false;
		schedule_job(state, runner);
	}
}

class CooperativeJobHandle final : public v8::JobHandle {
 public:
	CooperativeJobHandle(CooperativeTaskRunner* runner,
											 std::unique_ptr<v8::JobTask> task)
			: runner_(runner), state_(std::make_shared<JobState>(std::move(task))) {}

	void NotifyConcurrencyIncrease() override { schedule_job(state_, runner_); }

	void Join() override {
		state_->joining = true;
		state_->scheduled = false;
		while (job_is_active(*state_)) {
			run_job(state_, runner_);
		}
		state_->joining = false;
		state_->valid = false;
		state_->task.reset();
	}

	void Cancel() override {
		state_->cancelled = true;
		state_->valid = false;
		state_->task.reset();
	}

	void CancelAndDetach() override { Cancel(); }
	bool IsActive() override { return job_is_active(*state_); }
	bool IsValid() override { return state_->valid; }

 private:
	CooperativeTaskRunner* runner_;
	std::shared_ptr<JobState> state_;
};

class NoOpTracingController final : public v8::TracingController {};

class OsPlatform final : public v8::Platform {
 public:
	OsPlatform(v8::PageAllocator* page_allocator, bool use_kernel_allocator)
			: use_kernel_allocator_(use_kernel_allocator),
				external_page_allocator_(page_allocator),
				task_runner_(std::make_shared<CooperativeTaskRunner>()) {}

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
	int NumberOfWorkerThreads() override { return 0; }

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
			v8::TaskPriority, std::unique_ptr<v8::JobTask> task,
			const v8::SourceLocation&) override {
		return std::make_unique<CooperativeJobHandle>(task_runner_.get(),
																									 std::move(task));
	}

	bool PumpOne() { return task_runner_->PumpOne(); }
	uint64_t NextDeadlineNs() const { return task_runner_->NextDeadlineNs(); }

 protected:
	void PostTaskOnWorkerThreadImpl(v8::TaskPriority,
																	std::unique_ptr<v8::Task> task,
																	const v8::SourceLocation&) override {
		task_runner_->Enqueue(std::move(task), kz_host_time_ns());
	}

	void PostDelayedTaskOnWorkerThreadImpl(
			v8::TaskPriority, std::unique_ptr<v8::Task> task,
			double delay_in_seconds, const v8::SourceLocation&) override {
		task_runner_->Enqueue(
				std::move(task),
				deadline_from_delay(kz_host_time_ns(), delay_in_seconds));
	}

 private:
	bool use_kernel_allocator_;
#if defined(KZ_FREESTANDING)
	KernelPageAllocator kernel_page_allocator_;
#endif
	v8::PageAllocator* external_page_allocator_;
	std::shared_ptr<CooperativeTaskRunner> task_runner_;
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

extern "C" int kz_os_platform_pump_one(v8::Platform* platform) {
	return platform != nullptr && static_cast<OsPlatform*>(platform)->PumpOne();
}

extern "C" uint64_t kz_os_platform_next_deadline_ns(v8::Platform* platform) {
	return platform == nullptr ? kNoDeadline
															: static_cast<OsPlatform*>(platform)->NextDeadlineNs();
}
