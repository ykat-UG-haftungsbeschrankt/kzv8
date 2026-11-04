#include "os_platform.h"

#include <cstdio>
#include <cstdint>
#include <cerrno>
#include <atomic>
#include <memory>
#include <sys/mman.h>
#include <time.h>

#include "v8-context.h"
#include "v8-initialization.h"
#include "v8-isolate.h"
#include "v8-local-handle.h"
#include "v8-primitive.h"
#include "v8-script.h"

extern "C" uint64_t kz_host_time_ns() {
  timespec now{};
  clock_gettime(CLOCK_MONOTONIC, &now);
  return static_cast<uint64_t>(now.tv_sec) * 1'000'000'000 + now.tv_nsec;
}

extern "C" int64_t kz_host_wall_time_ms() {
  timespec now{};
  clock_gettime(CLOCK_REALTIME, &now);
  return static_cast<int64_t>(now.tv_sec) * 1000 + now.tv_nsec / 1'000'000;
}

namespace {

int host_protection(int permissions) {
  switch (permissions) {
    case 0:
    case 5:
      return PROT_NONE;
    case 1:
      return PROT_READ;
    case 2:
      return PROT_READ | PROT_WRITE;
    case 3:
      return PROT_READ | PROT_WRITE | PROT_EXEC;
    case 4:
      return PROT_READ | PROT_EXEC;
    default:
      return -1;
  }
}

class HostPageAllocator final : public v8::PageAllocator {
 public:
  size_t AllocatePageSize() override { return 4096; }
  size_t CommitPageSize() override { return 4096; }
  void SetRandomMmapSeed(int64_t) override {}

  void* GetRandomMmapAddr() override {
    constexpr size_t cage_size = size_t{1} << 32;
    void* reservation = mmap(nullptr, cage_size * 2, PROT_NONE,
                             MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE, -1,
                             0);
    if (reservation == MAP_FAILED) return nullptr;
    const uintptr_t start = reinterpret_cast<uintptr_t>(reservation);
    const uintptr_t aligned = (start + cage_size - 1) & ~(cage_size - 1);
    munmap(reservation, cage_size * 2);
    std::fprintf(stderr, "host random mmap hint=%p\n",
           reinterpret_cast<void*>(aligned));
    return reinterpret_cast<void*>(aligned);
  }

  void* AllocatePages(void* hint, size_t length, size_t alignment,
                      Permission permissions) override {
    std::fprintf(stderr, "host allocate len=%zu align=%zu perm=%d hint=%p\n",
           length, alignment, static_cast<int>(permissions), hint);
    std::fprintf(stderr, "host allocate len=%zu align=%zu perm=%d hint=%p\n",
           length, alignment, static_cast<int>(permissions), hint);
    const int protection = host_protection(static_cast<int>(permissions));
    if (protection < 0 || length == 0 ||
        length > static_cast<size_t>(-1) - alignment) {
      return nullptr;
    }
    const int flags = MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE;
    if (hint != nullptr) {
      void* exact = mmap(hint, length, protection, flags | MAP_FIXED_NOREPLACE,
                         -1, 0);
      if (exact != MAP_FAILED) {
        std::fprintf(stderr, "host exact mapping=%p\n", exact);
        return exact;
      }
      std::fprintf(stderr, "host exact mapping failed errno=%d\n", errno);
    }

    const size_t reserve_size = length + alignment;
    void* reservation = mmap(nullptr, reserve_size, PROT_NONE, flags, -1, 0);
    if (reservation == MAP_FAILED) {
      std::fprintf(stderr, "host reserve failed errno=%d\n", errno);
      return nullptr;
    }
    const uintptr_t start = reinterpret_cast<uintptr_t>(reservation);
    const uintptr_t aligned = (start + alignment - 1) & ~(alignment - 1);
    const size_t prefix = aligned - start;
    const size_t suffix = reserve_size - prefix - length;
    const int prefix_result = prefix == 0 ? 0 : munmap(reservation, prefix);
    const int suffix_result =
        suffix == 0 ? 0
                    : munmap(reinterpret_cast<void*>(aligned + length), suffix);
    const int permission_result =
        mprotect(reinterpret_cast<void*>(aligned), length, protection);
    if (prefix_result != 0 || suffix_result != 0 || permission_result != 0) {
      std::fprintf(stderr,
                   "host aligned mapping failed addr=%p len=%zu prefix=%zu "
                   "suffix=%zu errno=%d\n",
                   reinterpret_cast<void*>(aligned), length, prefix, suffix,
                   errno);
      munmap(reinterpret_cast<void*>(aligned), length);
      return nullptr;
    }
    std::fprintf(stderr, "host aligned mapping=%p len=%zu\n",
                 reinterpret_cast<void*>(aligned), length);
    return reinterpret_cast<void*>(aligned);
  }

  bool FreePages(void* address, size_t length) override {
    return munmap(address, length) == 0;
  }

  bool ReleasePages(void* address, size_t length,
                    size_t new_length) override {
    return new_length <= length &&
           (new_length == length ||
            munmap(static_cast<char*>(address) + new_length,
                   length - new_length) == 0);
  }

  bool SetPermissions(void* address, size_t length,
                      Permission permissions) override {
    std::fprintf(stderr, "host protect addr=%p len=%zu perm=%d\n", address,
           length, static_cast<int>(permissions));
    std::fprintf(stderr, "host protect addr=%p len=%zu perm=%d\n", address,
           length, static_cast<int>(permissions));
    const int protection = host_protection(static_cast<int>(permissions));
    return protection >= 0 && mprotect(address, length, protection) == 0;
  }

  bool DecommitPages(void* address, size_t length) override {
    return madvise(address, length, MADV_DONTNEED) == 0 &&
           mprotect(address, length, PROT_NONE) == 0;
  }

  bool RecommitPages(void* address, size_t length,
                     Permission permissions) override {
    const int protection = host_protection(static_cast<int>(permissions));
    return protection >= 0 && mprotect(address, length, protection) == 0;
  }
};

class IncrementTask final : public v8::Task {
 public:
  explicit IncrementTask(int* count) : count_(count) {}
  void Run() override { ++*count_; }

 private:
  int* count_;
};

class WorkerIncrementTask final : public v8::Task {
 public:
  explicit WorkerIncrementTask(std::atomic<int>* count) : count_(count) {}
  void Run() override { count_->fetch_add(1, std::memory_order_release); }

 private:
  std::atomic<int>* count_;
};

class CountingJob final : public v8::JobTask {
 public:
  CountingJob(std::atomic<int>* count, size_t work)
      : count_(count), remaining_(work) {}

  void Run(v8::JobDelegate* delegate) override {
    while (!delegate->ShouldYield()) {
      size_t remaining = remaining_.load(std::memory_order_relaxed);
      while (remaining != 0 &&
             !remaining_.compare_exchange_weak(remaining, remaining - 1,
                                               std::memory_order_relaxed)) {
      }
      if (remaining == 0) return;
      count_->fetch_add(1, std::memory_order_relaxed);
    }
  }

  size_t GetMaxConcurrency(size_t) const override {
    return remaining_.load(std::memory_order_relaxed);
  }

 private:
  std::atomic<int>* count_;
  std::atomic<size_t> remaining_;
};

int fail(const char* message) {
  std::fprintf(stderr, "FAIL: %s\n", message);
  return 1;
}

}  // namespace

int main() {
  std::fprintf(stderr, "phase: flags\n");
  HostPageAllocator host_page_allocator;
  v8::Platform* platform = kz_os_platform_create(&host_page_allocator);
  if (platform == nullptr) return fail("platform allocation");
  std::fprintf(stderr, "host allocator=%p platform allocator=%p\n",
               static_cast<void*>(&host_page_allocator),
               static_cast<void*>(platform->GetPageAllocator()));
  std::fprintf(stderr, "phase: platform\n");
  v8::V8::InitializePlatform(platform);
  std::fprintf(stderr, "phase: initialize\n");
  if (!v8::V8::Initialize()) return fail("V8 initialization");
  if (platform->NumberOfWorkerThreads() == 0) {
    return fail("worker thread startup");
  }

  std::fprintf(stderr, "phase: isolate\n");
  v8::ArrayBuffer::Allocator* allocator =
      v8::ArrayBuffer::Allocator::NewDefaultAllocator();
  std::fprintf(stderr, "phase: array-buffer-allocator\n");
  v8::Isolate::CreateParams params;
  params.array_buffer_allocator = allocator;
  std::fprintf(stderr, "phase: isolate-create\n");
  v8::Isolate* isolate = v8::Isolate::New(params);
  std::fprintf(stderr, "phase: isolate-created\n");
  if (isolate == nullptr) return fail("isolate creation");

  {
    v8::Isolate::Scope isolate_scope(isolate);
    v8::HandleScope handle_scope(isolate);
    v8::Local<v8::Context> context = v8::Context::New(isolate);
    v8::Context::Scope context_scope(context);
    v8::Local<v8::String> source =
        v8::String::NewFromUtf8Literal(isolate, "6 * 7");
    v8::Local<v8::Script> script;
    if (!v8::Script::Compile(context, source).ToLocal(&script)) {
      return fail("script compile");
    }
    v8::Local<v8::Value> result;
    if (!script->Run(context).ToLocal(&result) ||
        result->Int32Value(context).FromMaybe(-1) != 42) {
      return fail("script execution");
    }
  }
  std::fprintf(stderr, "phase: script\n");

  int task_count = 0;
  std::shared_ptr<v8::TaskRunner> runner =
      platform->GetForegroundTaskRunner(isolate);
  runner->PostTask(std::make_unique<IncrementTask>(&task_count));
  if (task_count != 0 || !kz_os_platform_pump_one(platform) || task_count != 1) {
    return fail("foreground task pumping");
  }
  std::fprintf(stderr, "phase: foreground\n");

  std::atomic<int> worker_task_count{0};
  platform->PostTaskOnWorkerThread(
      v8::TaskPriority::kUserVisible,
      std::make_unique<WorkerIncrementTask>(&worker_task_count));
  const uint64_t worker_deadline = kz_host_time_ns() + 1'000'000'000;
  while (worker_task_count.load(std::memory_order_acquire) == 0 &&
         kz_host_time_ns() < worker_deadline) {
  }
  if (worker_task_count.load(std::memory_order_acquire) != 1) {
    return fail("preemptive worker task execution");
  }
  std::fprintf(stderr, "phase: worker\n");

  runner->PostDelayedTask(std::make_unique<IncrementTask>(&task_count), 0.001);
  const uint64_t deadline = kz_os_platform_next_deadline_ns(platform);
  if (deadline == UINT64_MAX || kz_os_platform_pump_one(platform)) {
    return fail("delayed task deadline");
  }
  while (kz_host_time_ns() < deadline) {
  }
  if (!kz_os_platform_pump_one(platform) || task_count != 2) {
    return fail("delayed task pumping");
  }
  std::fprintf(stderr, "phase: delayed\n");

  std::atomic<int> job_count{0};
  auto job = platform->PostJob(v8::TaskPriority::kUserVisible,
                               std::make_unique<CountingJob>(&job_count, 4));
  job->Join();
  if (job_count.load(std::memory_order_relaxed) != 4 || job->IsValid()) {
    return fail("cooperative job execution");
  }
  std::fprintf(stderr, "phase: jobs\n");

  isolate->Dispose();
  delete allocator;
  v8::V8::Dispose();
  v8::V8::DisposePlatform();
  std::fprintf(stderr, "phase: disposed\n");
  kz_os_platform_destroy(platform);
  std::puts("PASS: custom x64 V8 platform, JS, tasks, timers, jobs");
  return 0;
}